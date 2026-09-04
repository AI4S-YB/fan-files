import { useEffect, useState } from "react";

/** 监听元素滚动：>threshold 触发 `scrolled=true`。
 *  给粘性 header 加阴影渐显用。
 */
export function useScrollShadow(threshold = 8): {
  ref: (el: HTMLElement | null) => void;
  scrolled: boolean;
} {
  const [scrolled, setScrolled] = useState(false);
  const [el, setEl] = useState<HTMLElement | null>(null);

  useEffect(() => {
    if (!el) return;
    const onScroll = () => setScrolled(el.scrollTop > threshold);
    onScroll();
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, [el, threshold]);

  return { ref: setEl, scrolled };
}
