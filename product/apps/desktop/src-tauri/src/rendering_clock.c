/* AG1-local compatibility for drivers without DRM_WAIT_VBLANK (notably NVIDIA).
 * Preserve real libdrm behavior whenever it works. For unsupported synchronous
 * relative waits only, use GTK's Wayland frame clock, never a fixed-rate timer.
 * Linked into AG1's executable only: no preload or system-library replacement.
 */
#define _GNU_SOURCE
#include <gtk/gtk.h>
#include <xf86drm.h>
#include <dlfcn.h>
#include <errno.h>
#include <pthread.h>
#include <stdatomic.h>
#include <string.h>
#include <time.h>
#include <sys/time.h>

static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t ready;
static pthread_once_t initialized = PTHREAD_ONCE_INIT;
static int (*original_wait)(int, drmVBlankPtr);
static _Atomic int enabled;
static GtkWidget *clock_widget; /* Accessed only on GTK's main thread. */
static unsigned sequence, tick_id;
static int start_pending, announced;
static gint64 last_request;

static void initialize(void) {
    original_wait = dlsym(RTLD_NEXT, "drmWaitVBlank");
    pthread_condattr_t attr;
    pthread_condattr_init(&attr);
    pthread_condattr_setclock(&attr, CLOCK_MONOTONIC);
    pthread_cond_init(&ready, &attr);
    pthread_condattr_destroy(&attr);
}

void ag1_frame_clock_initialize(int candidate) {
    pthread_once(&initialized, initialize);
    atomic_store(&enabled, candidate);
}

static gboolean frame_tick(GtkWidget *widget, GdkFrameClock *clock, gpointer unused) {
    (void)widget; (void)clock; (void)unused;
    pthread_mutex_lock(&lock);
    if (!atomic_load(&enabled) || g_get_monotonic_time() - last_request > 250000) {
        tick_id = 0;
        pthread_mutex_unlock(&lock);
        return G_SOURCE_REMOVE;
    }
    sequence++;
    pthread_cond_broadcast(&ready);
    pthread_mutex_unlock(&lock);
    return G_SOURCE_CONTINUE;
}

static gboolean start_clock(gpointer unused) {
    (void)unused;
    pthread_mutex_lock(&lock);
    start_pending = 0;
    if (clock_widget && atomic_load(&enabled) && !tick_id)
        tick_id = gtk_widget_add_tick_callback(clock_widget, frame_tick, NULL, NULL);
    pthread_mutex_unlock(&lock);
    return G_SOURCE_REMOVE;
}

static void widget_destroyed(GtkWidget *widget, gpointer unused) {
    (void)widget; (void)unused;
    pthread_mutex_lock(&lock);
    atomic_store(&enabled, 0);
    clock_widget = NULL;
    tick_id = 0;
    pthread_cond_broadcast(&ready);
    pthread_mutex_unlock(&lock);
}

void ag1_frame_clock_attach(GtkWidget *widget) {
    pthread_once(&initialized, initialize);
    if (!atomic_load(&enabled)) return;
    GdkDisplay *display = gtk_widget_get_display(widget);
    if (!display || strcmp(G_OBJECT_TYPE_NAME(display), "GdkWaylandDisplay")) {
        atomic_store(&enabled, 0);
        return;
    }
    clock_widget = widget;
    g_signal_connect(widget, "destroy", G_CALLBACK(widget_destroyed), NULL);
}

/* Test/diagnostic observation; neither this nor the adapter supplies a renderer
 * command, frame-rate override, or additional device permissions. */
unsigned ag1_frame_clock_frames(void) {
    pthread_mutex_lock(&lock);
    unsigned value = sequence;
    pthread_mutex_unlock(&lock);
    return value;
}

int drmWaitVBlank(int fd, drmVBlankPtr value) {
    pthread_once(&initialized, initialize);
    if (!original_wait) { errno = ENOSYS; return -1; }
    if (!value) return original_wait(fd, value);
    /* libdrm/the failed ioctl can overwrite the request union. Capture first. */
    unsigned type = value->request.type, amount = value->request.sequence;
    unsigned long signal = value->request.signal;
    int result = original_wait(fd, value);
    if (!result || errno != ENOTSUP || !atomic_load(&enabled) || signal
        || (type & ~(DRM_VBLANK_HIGH_CRTC_MASK | DRM_VBLANK_SECONDARY)) != DRM_VBLANK_RELATIVE
        || amount > 1)
        return result;

    if (!amount) {
        GdkDisplay *display = gdk_display_get_default();
        if (!display || strcmp(G_OBJECT_TYPE_NAME(display), "GdkWaylandDisplay")) return result;
    }
    pthread_mutex_lock(&lock);
    if (!announced) {
        announced = 1;
        g_printerr("AG1 rendering: DRM refresh wait unsupported; using the Wayland GTK frame clock\n");
    }
    unsigned before = sequence;
    if (amount) {
        last_request = g_get_monotonic_time();
        if (!tick_id && !start_pending) {
            start_pending = 1;
            /* Always schedule, including when called by GTK's owner thread. */
            g_idle_add_full(G_PRIORITY_HIGH_IDLE, start_clock, NULL, NULL);
        }
        struct timespec until;
        clock_gettime(CLOCK_MONOTONIC, &until);
        until.tv_sec++;
        while (before == sequence && atomic_load(&enabled)) {
            if (pthread_cond_timedwait(&ready, &lock, &until) == ETIMEDOUT) break;
        }
    }
    value->reply.type = type;
    value->reply.sequence = sequence;
    /* A hidden/unmapped window may have no ticks. Do not permanently disable
     * WebKit's monitor on that pause; its observer lifecycle controls restarting. */
    pthread_mutex_unlock(&lock);
    struct timeval now;
    gettimeofday(&now, NULL);
    value->reply.tval_sec = now.tv_sec;
    value->reply.tval_usec = now.tv_usec;
    return 0;
}
