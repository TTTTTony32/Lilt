import { useCallback, useEffect, useRef, useState, type AnimationEvent as ReactAnimationEvent, type MouseEvent as ReactMouseEvent, type ReactNode } from "react";
import { usePrefersReducedMotion } from "./usePrefersReducedMotion";

type OverlayPhase = "opening" | "open" | "closing";

export function AnimatedOverlay({
  open,
  className,
  children,
  onClosed,
  onBackdropClick,
}: {
  open: boolean;
  className: string;
  children: ReactNode;
  onClosed: () => void;
  onBackdropClick?: (event: ReactMouseEvent<HTMLDivElement>) => void;
}) {
  const reducedMotion = usePrefersReducedMotion();
  const [phase, setPhase] = useState<OverlayPhase>(open ? "opening" : "closing");
  const closeNotifiedRef = useRef(false);
  const overlayRef = useRef<HTMLDivElement | null>(null);

  const notifyClosed = useCallback(() => {
    if (closeNotifiedRef.current) return;
    closeNotifiedRef.current = true;
    onClosed();
  }, [onClosed]);

  useEffect(() => {
    if (open) {
      closeNotifiedRef.current = false;
      setPhase("opening");
      if (reducedMotion) {
        setPhase("open");
        return;
      }
      return undefined;
    }

    setPhase("closing");
    const activeElement = document.activeElement;
    if (activeElement instanceof HTMLElement && overlayRef.current?.contains(activeElement)) {
      activeElement.blur();
    }
    if (reducedMotion) notifyClosed();
    return undefined;
  }, [notifyClosed, open, reducedMotion]);

  const handleAnimationEnd = useCallback((event: ReactAnimationEvent<HTMLDivElement>) => {
    if (event.target !== event.currentTarget) return;
    if (open && phase === "opening") {
      setPhase("open");
      return;
    }
    if (!open && phase === "closing") notifyClosed();
  }, [notifyClosed, open, phase]);

  return (
    <div
      ref={overlayRef}
      className={`${className} overlay-phase-${phase}`}
      role="presentation"
      aria-hidden={phase === "closing"}
      onClick={onBackdropClick}
      onAnimationEnd={handleAnimationEnd}
    >
      {children}
    </div>
  );
}
