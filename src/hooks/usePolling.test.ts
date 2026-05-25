import { renderHook, act } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { usePolling } from "./usePolling";

describe("usePolling", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("fetches immediately when enabled", async () => {
    const fetchFn = vi.fn().mockResolvedValue("result");

    renderHook(() => usePolling(fetchFn, 10000, true));

    expect(fetchFn).toHaveBeenCalledTimes(1);
  });

  it("returns data after successful fetch", async () => {
    const fetchFn = vi.fn().mockResolvedValue("hello");

    const { result } = renderHook(() => usePolling(fetchFn, 10000, true));

    await act(async () => {
      await vi.runOnlyPendingTimersAsync();
    });

    expect(result.current.data).toBe("hello");
    expect(result.current.error).toBeNull();
    expect(result.current.stale).toBe(false);
    expect(result.current.loading).toBe(false);
  });

  it("polls at the specified interval", async () => {
    const fetchFn = vi.fn().mockResolvedValue("data");

    renderHook(() => usePolling(fetchFn, 5000, true));

    // Initial fetch
    expect(fetchFn).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });

    expect(fetchFn).toHaveBeenCalledTimes(2);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });

    expect(fetchFn).toHaveBeenCalledTimes(3);
  });

  it("does not fetch when disabled", () => {
    const fetchFn = vi.fn().mockResolvedValue("data");

    renderHook(() => usePolling(fetchFn, 10000, false));

    expect(fetchFn).not.toHaveBeenCalled();
  });

  it("stops polling when enabled becomes false", async () => {
    const fetchFn = vi.fn().mockResolvedValue("data");

    const { rerender } = renderHook(
      ({ enabled }) => usePolling(fetchFn, 5000, enabled),
      { initialProps: { enabled: true } },
    );

    expect(fetchFn).toHaveBeenCalledTimes(1);

    // Disable polling
    rerender({ enabled: false });

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    // Should not have been called again after disabling
    expect(fetchFn).toHaveBeenCalledTimes(1);
  });

  it("starts polling when enabled becomes true", async () => {
    const fetchFn = vi.fn().mockResolvedValue("data");

    const { rerender } = renderHook(
      ({ enabled }) => usePolling(fetchFn, 5000, enabled),
      { initialProps: { enabled: false } },
    );

    expect(fetchFn).not.toHaveBeenCalled();

    // Enable polling
    rerender({ enabled: true });

    expect(fetchFn).toHaveBeenCalledTimes(1);
  });

  it("sets stale to true on fetch failure but retains last data", async () => {
    const fetchFn = vi
      .fn()
      .mockResolvedValueOnce("good-data")
      .mockRejectedValueOnce(new Error("network error"));

    const { result } = renderHook(() => usePolling(fetchFn, 5000, true));

    // Initial fetch is called synchronously, wait for promise to resolve
    await act(async () => {
      await Promise.resolve();
    });

    expect(fetchFn).toHaveBeenCalledTimes(1);
    expect(result.current.data).toBe("good-data");
    expect(result.current.stale).toBe(false);

    // Advance to trigger second fetch (interval)
    await act(async () => {
      vi.advanceTimersByTime(5000);
      await Promise.resolve();
    });

    expect(fetchFn).toHaveBeenCalledTimes(2);
    expect(result.current.data).toBe("good-data"); // retained
    expect(result.current.stale).toBe(true);
    expect(result.current.error).toBeInstanceOf(Error);
    expect(result.current.error?.message).toBe("network error");
  });

  it("clears stale and error on subsequent successful fetch", async () => {
    const fetchFn = vi
      .fn()
      .mockResolvedValueOnce("first")
      .mockRejectedValueOnce(new Error("fail"))
      .mockResolvedValueOnce("recovered");

    const { result } = renderHook(() => usePolling(fetchFn, 5000, true));

    // First fetch succeeds
    await act(async () => {
      await Promise.resolve();
    });

    expect(result.current.data).toBe("first");
    expect(result.current.stale).toBe(false);

    // Second fetch fails
    await act(async () => {
      vi.advanceTimersByTime(5000);
      await Promise.resolve();
    });

    expect(result.current.stale).toBe(true);

    // Third fetch succeeds
    await act(async () => {
      vi.advanceTimersByTime(5000);
      await Promise.resolve();
    });

    expect(result.current.data).toBe("recovered");
    expect(result.current.stale).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it("refresh() triggers immediate fetch and resets interval", async () => {
    const fetchFn = vi.fn().mockResolvedValue("data");

    const { result } = renderHook(() => usePolling(fetchFn, 10000, true));

    // Initial fetch called synchronously
    expect(fetchFn).toHaveBeenCalledTimes(1);

    await act(async () => {
      await Promise.resolve();
    });

    // Advance 7 seconds (not yet at 10s interval)
    await act(async () => {
      vi.advanceTimersByTime(7000);
    });

    // Still only the initial fetch
    expect(fetchFn).toHaveBeenCalledTimes(1);

    // Call refresh — triggers immediate fetch and resets interval
    await act(async () => {
      result.current.refresh();
      await Promise.resolve();
    });

    // Should have fetched immediately (initial + refresh = 2)
    expect(fetchFn).toHaveBeenCalledTimes(2);

    // Advance 7 seconds — should NOT trigger another fetch
    // because the interval was reset from the refresh point
    await act(async () => {
      vi.advanceTimersByTime(7000);
      await Promise.resolve();
    });

    expect(fetchFn).toHaveBeenCalledTimes(2);

    // Advance 3 more seconds (total 10s from refresh) — should trigger
    await act(async () => {
      vi.advanceTimersByTime(3000);
      await Promise.resolve();
    });

    expect(fetchFn).toHaveBeenCalledTimes(3);
  });

  it("refresh() does nothing when disabled", async () => {
    const fetchFn = vi.fn().mockResolvedValue("data");

    const { result } = renderHook(() => usePolling(fetchFn, 10000, false));

    await act(async () => {
      result.current.refresh();
    });

    expect(fetchFn).not.toHaveBeenCalled();
  });

  it("loading is true only during initial fetch", async () => {
    const fetchFn = vi.fn().mockResolvedValue("data");

    const { result } = renderHook(() => usePolling(fetchFn, 10000, true));

    // Before first fetch resolves, loading should be true
    expect(result.current.loading).toBe(true);

    await act(async () => {
      await vi.runOnlyPendingTimersAsync();
    });

    // After first fetch resolves, loading should be false
    expect(result.current.loading).toBe(false);
  });

  it("loading is false when disabled", () => {
    const fetchFn = vi.fn().mockResolvedValue("data");

    const { result } = renderHook(() => usePolling(fetchFn, 10000, false));

    expect(result.current.loading).toBe(false);
  });

  it("handles non-Error thrown values", async () => {
    const fetchFn = vi.fn().mockRejectedValue("string error");

    const { result } = renderHook(() => usePolling(fetchFn, 10000, true));

    await act(async () => {
      await vi.runOnlyPendingTimersAsync();
    });

    expect(result.current.error).toBeInstanceOf(Error);
    expect(result.current.error?.message).toBe("string error");
    expect(result.current.stale).toBe(true);
  });
});
