import "./activityMotion.css";

/** AG1's activity mark. Change the stroke, rotations and viewBox here; motion,
 * size and theme live in activityMotion.css. No timers or native dependencies. */
export function ActivityMark({ paused = false }: { paused?: boolean }) {
  return <svg className="ag1-activity-mark" data-paused={paused || undefined} viewBox="0 0 24 24" aria-hidden="true" focusable="false">
    {[0, 120, 240].map(angle => <g key={angle} transform={`rotate(${angle} 12 12)`}><path d="M 9 5 L 15 5" /></g>)}
  </svg>;
}
