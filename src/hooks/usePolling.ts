import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Custom hook that polls a fetch function at a specified interval.
 *
 * - `enabled` controls whether polling is active (tied to tab visibility)
 * - `refresh()` triggers an immediate fetch AND resets the interval timer
 * - On fetch failure: sets `stale: true` but keeps last successful `data`
 * - On fetch success: sets `stale: false`, updates `data`
 * - `loading` is true only during the initial fetch (when data is null)
 */
export function usePolling<T>(
  fetchFn: () => Promise<T>,
  intervalMs: number,
  enabled: boolean,
): {
  data: T | null;
  error: Error | null;
  loading: boolean;
  stale: boolean;
  refresh: () => void;
} {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<Error | null>(null);
  const [stale, setStale] = useState(false);

  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const fetchFnRef = useRef(fetchFn);
  const isMountedRef = useRef(true);
  const hasFetchedRef = useRef(false);

  // Keep fetchFn ref current without triggering effect re-runs
  useEffect(() => {
    fetchFnRef.current = fetchFn;
  }, [fetchFn]);

  const clearPollingInterval = useCallback(() => {
    if (intervalRef.current !== null) {
      clearInterval(intervalRef.current);
      intervalRef.current = null;
    }
  }, []);

  const doFetch = useCallback(async () => {
    try {
      const result = await fetchFnRef.current();
      if (!isMountedRef.current) return;
      setData(result);
      setError(null);
      setStale(false);
      hasFetchedRef.current = true;
    } catch (err) {
      if (!isMountedRef.current) return;
      const e = err instanceof Error ? err : new Error(String(err));
      setError(e);
      setStale(true);
      hasFetchedRef.current = true;
    }
  }, []);

  const startPolling = useCallback(() => {
    clearPollingInterval();
    // Fetch immediately when polling starts
    doFetch();
    // Set up interval for subsequent fetches
    intervalRef.current = setInterval(doFetch, intervalMs);
  }, [doFetch, intervalMs, clearPollingInterval]);

  const refresh = useCallback(() => {
    if (!enabled) return;
    // Trigger immediate fetch and reset the interval timer
    clearPollingInterval();
    doFetch();
    intervalRef.current = setInterval(doFetch, intervalMs);
  }, [enabled, doFetch, intervalMs, clearPollingInterval]);

  // Start/stop polling based on `enabled`
  useEffect(() => {
    if (enabled) {
      startPolling();
    } else {
      clearPollingInterval();
    }

    return () => {
      clearPollingInterval();
    };
  }, [enabled, startPolling, clearPollingInterval]);

  // Cleanup on unmount
  useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
    };
  }, []);

  // loading is true only during the initial fetch (when data is null and no fetch has completed)
  const loading = enabled && !hasFetchedRef.current && data === null;

  return { data, error, loading, stale, refresh };
}
