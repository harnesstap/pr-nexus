/**
 * One timer at a time. The callback runs after `ms`, then the next wait is
 * armed. A sleep that skips several intervals can fire the pending timer
 * only once; it cannot replay a tick per missed interval the way
 * `setInterval` does after the machine wakes.
 */
export function every(ms: number, tick: () => void): () => void {
  let timer = 0;
  let stopped = false;
  const arm = () => {
    timer = window.setTimeout(() => {
      if (stopped) {
        return;
      }
      tick();
      arm();
    }, ms);
  };
  arm();
  return () => {
    stopped = true;
    window.clearTimeout(timer);
  };
}
