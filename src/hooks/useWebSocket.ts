import { useCallback, useEffect, useRef, useState } from "react";

export enum ReadyState {
  CONNECTING = 0,
  OPEN = 1,
  CLOSING = 2,
  CLOSED = 3,
}

interface UseWebSocketOptions {
  reconnect?: boolean;
  reconnectInterval?: number;
  maxReconnectAttempts?: number;
}

interface UseWebSocketReturn {
  sendMessage: (data: string | ArrayBuffer) => void;
  lastMessage: MessageEvent | null;
  readyState: ReadyState;
  connect: () => void;
  disconnect: () => void;
}

export function useWebSocket(
  url: string | null,
  options: UseWebSocketOptions = {},
): UseWebSocketReturn {
  const {
    reconnect = true,
    reconnectInterval = 3000,
    maxReconnectAttempts = 5,
  } = options;

  const [lastMessage, setLastMessage] = useState<MessageEvent | null>(null);
  const [readyState, setReadyState] = useState<ReadyState>(ReadyState.CLOSED);
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectCount = useRef(0);
  const reconnectTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const unmountedRef = useRef(false);

  const cleanup = useCallback(() => {
    if (reconnectTimer.current) {
      clearTimeout(reconnectTimer.current);
      reconnectTimer.current = null;
    }
    if (wsRef.current) {
      wsRef.current.onopen = null;
      wsRef.current.onmessage = null;
      wsRef.current.onclose = null;
      wsRef.current.onerror = null;
      wsRef.current.close();
      wsRef.current = null;
    }
  }, []);

  const connect = useCallback(() => {
    if (!url) return;
    cleanup();
    setReadyState(ReadyState.CONNECTING);

    const ws = new WebSocket(url);
    wsRef.current = ws;

    ws.onopen = () => {
      if (unmountedRef.current) return;
      reconnectCount.current = 0;
      setReadyState(ReadyState.OPEN);
    };

    ws.onmessage = (event: MessageEvent) => {
      if (unmountedRef.current) return;
      setLastMessage(event);
    };

    ws.onclose = () => {
      if (unmountedRef.current) return;
      setReadyState(ReadyState.CLOSED);
      if (reconnect && reconnectCount.current < maxReconnectAttempts) {
        const delay = Math.min(
          reconnectInterval * Math.pow(2, reconnectCount.current),
          30_000,
        ) + Math.random() * 1000;
        reconnectCount.current += 1;
        reconnectTimer.current = setTimeout(connect, delay);
      }
    };

    ws.onerror = () => {
      if (unmountedRef.current) return;
      ws.close();
    };
  }, [url, reconnect, reconnectInterval, maxReconnectAttempts, cleanup]);

  const disconnect = useCallback(() => {
    reconnectCount.current = maxReconnectAttempts;
    cleanup();
    setReadyState(ReadyState.CLOSED);
  }, [cleanup, maxReconnectAttempts]);

  const sendMessage = useCallback((data: string | ArrayBuffer) => {
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(data);
    }
  }, []);

  useEffect(() => {
    unmountedRef.current = false;
    return () => {
      unmountedRef.current = true;
      cleanup();
    };
  }, [cleanup]);

  return { sendMessage, lastMessage, readyState, connect, disconnect };
}
