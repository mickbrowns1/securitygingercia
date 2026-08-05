import { useEffect, useRef } from "react";

// Runs `callback` immediately, then every `delayMs` -- `callback` is read
// from a ref on each tick so the interval itself never needs recreating
// just because the callback closure changed (e.g. captures freshly
// changed state), matching the always-current-closure behavior the
// original vanilla setInterval callbacks relied on implicitly.
export function useInterval(callback, delayMs, enabled = true) {
  const callbackRef = useRef(callback);
  callbackRef.current = callback;

  useEffect(() => {
    if (!enabled) return;
    callbackRef.current();
    const id = setInterval(() => callbackRef.current(), delayMs);
    return () => clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [delayMs, enabled]);
}
