import { useCallback, useEffect, useRef, useState } from "react";

interface ResizeHandleProps {
  /** Ref to the element being resized */
  targetRef: React.RefObject<HTMLElement | null>;
  /** Minimum width in pixels */
  minWidth?: number;
  /** Maximum width in pixels */
  maxWidth?: number;
  /** Called when resize completes with the new width */
  onResize?: (width: number) => void;
}

export function ResizeHandle({
  targetRef,
  minWidth = 120,
  maxWidth = 400,
  onResize,
}: ResizeHandleProps) {
  const [dragging, setDragging] = useState(false);
  const startX = useRef(0);
  const startWidth = useRef(0);

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      if (!targetRef.current) return;
      setDragging(true);
      startX.current = e.clientX;
      startWidth.current = targetRef.current.offsetWidth;
    },
    [targetRef]
  );

  useEffect(() => {
    if (!dragging) return;

    const handleMouseMove = (e: MouseEvent) => {
      if (!targetRef.current) return;
      const delta = e.clientX - startX.current;
      const newWidth = Math.min(maxWidth, Math.max(minWidth, startWidth.current + delta));
      targetRef.current.style.width = `${newWidth}px`;
    };

    const handleMouseUp = () => {
      setDragging(false);
      if (targetRef.current && onResize) {
        onResize(targetRef.current.offsetWidth);
      }
    };

    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleMouseUp);
    return () => {
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
    };
  }, [dragging, targetRef, minWidth, maxWidth, onResize]);

  return (
    <div
      className={`resize-handle ${dragging ? "dragging" : ""}`}
      onMouseDown={handleMouseDown}
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize panel"
    />
  );
}
