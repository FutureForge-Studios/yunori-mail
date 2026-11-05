import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Load-on-mount data hook. `key` is the dependency: change it to refetch.
 * Stale responses (an older key, or after unmount) are dropped.
 */
export function useApi<T>(key: string | null, fn: () => Promise<T>) {
  const [data, setData] = useState<T | undefined>(undefined);
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(key !== null);
  const fnRef = useRef(fn);
  fnRef.current = fn;
  const gen = useRef(0);

  const load = useCallback(async () => {
    const mine = ++gen.current;
    setLoading(true);
    try {
      const d = await fnRef.current();
      if (mine === gen.current) {
        setData(d);
        setError(null);
      }
    } catch (err) {
      if (mine === gen.current) setError(err);
    } finally {
      if (mine === gen.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (key === null) return;
    void load();
    const g = gen; // invalidate in-flight responses for the old key
    return () => {
      g.current++;
    };
  }, [key, load]);

  return { data, error, loading, reload: load, setData };
}
