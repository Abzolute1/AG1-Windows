#define _GNU_SOURCE
#include <malloc.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <time.h>
#include <errno.h>
static pthread_t worker;
static pthread_mutex_t lock=PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t wake;
static _Atomic int stopping=0;
static FILE *output;
static unsigned interval=60;
static int started;
static void sample(void) {
    char *xml=NULL;size_t bytes=0;
    FILE *memory=open_memstream(&xml,&bytes);
    if (!memory) return;
    int result=malloc_info(0,memory);fclose(memory);
    struct timespec now;clock_gettime(CLOCK_MONOTONIC,&now);
    uint64_t ns=(uint64_t)now.tv_sec*1000000000ull+now.tv_nsec;
    fprintf(output,"{\"monotonic_ns\":%llu,\"pid\":%ld,\"malloc_info_result\":%d,\"xml\":\"",(unsigned long long)ns,(long)getpid(),result);
    for(size_t i=0;i<bytes;i++) {
        unsigned char ch=xml[i];
        if(ch=='\n') fputs("\\n",output);
        else if(ch=='\r') fputs("\\r",output);
        else if(ch=='\t') fputs("\\t",output);
        else if(ch=='"' || ch=='\\') {fputc('\\',output);fputc(ch,output);}
        else if(ch<32) fprintf(output,"\\u%04x",ch);
        else fputc(ch,output);
    }
    fputs("\"}\n",output);fflush(output);fsync(fileno(output));free(xml);
}
static void *observe(void *unused) {
    (void)unused;
    while(!atomic_load(&stopping)) {
        sample();
        struct timespec until;clock_gettime(CLOCK_MONOTONIC,&until);until.tv_sec+=interval;
        pthread_mutex_lock(&lock);
        if(!atomic_load(&stopping)) pthread_cond_timedwait(&wake,&lock,&until);
        pthread_mutex_unlock(&lock);
    }
    sample();return NULL;
}
__attribute__((constructor)) static void begin(void) {
    const char *prefix=getenv("AG1_HEAP_SAMPLE_PREFIX");
    if(!prefix || prefix[0]!='/' || strlen(prefix)>3500) return;
    const char *seconds=getenv("AG1_HEAP_SAMPLE_SECONDS");
    if(seconds) {char *end=NULL;unsigned long n=strtoul(seconds,&end,10);if(!end || *end || !n || n>60) return;interval=n;}
    char name[4096];snprintf(name,sizeof(name),"%s.%ld.jsonl",prefix,(long)getpid());
    int fd=open(name,O_WRONLY|O_CREAT|O_EXCL|O_CLOEXEC,0600);if(fd<0)return;
    output=fdopen(fd,"w");if(!output){close(fd);return;}
    pthread_condattr_t attr;
    if(pthread_condattr_init(&attr)) {fclose(output);output=NULL;return;}
    int condition=pthread_condattr_setclock(&attr,CLOCK_MONOTONIC);
    if(!condition) condition=pthread_cond_init(&wake,&attr);
    pthread_condattr_destroy(&attr);
    if(condition) {fclose(output);output=NULL;return;}
    if(pthread_create(&worker,NULL,observe,NULL)) {fclose(output);output=NULL;return;}
    started=1;
}
__attribute__((destructor)) static void finish(void) {
    if(!started) return;
    pthread_mutex_lock(&lock);atomic_store(&stopping,1);pthread_cond_signal(&wake);pthread_mutex_unlock(&lock);
    pthread_join(worker,NULL);pthread_cond_destroy(&wake);fclose(output);output=NULL;
}
