import { useEffect } from "react";
import { completeOverlayHide, onOverlayEvent } from "./bridge";

/** Keep the native window visible until the pill finishes its exit animation. */
export function useOverlayVisibility(ref: React.RefObject<HTMLDivElement | null>) {
  useEffect(() => {
    let animation: Animation | undefined;
    let visible = false;
    let generation = 0;
    const unlisten = onOverlayEvent((event) => {
      if (event.type !== "visibility" || !ref.current) return;
      const nextVisible = event.payload.visible === true;
      const nextGeneration = event.payload.generation as number;
      const pill = ref.current;
      // Read the displayed frame before cancelling an interrupted animation.
      const current = getComputedStyle(pill);
      const from = visible || animation ? { opacity: current.opacity } : { opacity: "0" };
      animation?.cancel();
      visible = nextVisible;
      generation = nextGeneration;
      const target = nextVisible ? { opacity: "1" } : { opacity: "0" };
      pill.style.opacity = target.opacity;
      const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
      animation = pill.animate([from, target], {
        duration: reduced ? 0 : nextVisible ? 160 : 120,
        easing: nextVisible ? "ease-out" : "ease-in",
      });
      animation.finished
        .then(() => {
          if (!nextVisible && generation === nextGeneration) {
            void completeOverlayHide(nextGeneration).catch(() => {});
          }
          if (generation === nextGeneration) animation = undefined;
        })
        .catch(() => {}); // Cancelling an animation rejects its finished promise.
    });
    return () => {
      unlisten();
      generation = -1;
      animation?.cancel();
    };
  }, [ref]);
}
