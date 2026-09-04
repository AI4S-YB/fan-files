import { describe, it, expect } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useScrollShadow } from "./useScrollShadow";

describe("useScrollShadow", () => {
  it("returns scrolled=false initially", () => {
    const { result } = renderHook(() => useScrollShadow());
    expect(result.current.scrolled).toBe(false);
  });

  it("toggles scrolled when element scrolls past threshold", () => {
    const { result } = renderHook(() => useScrollShadow(8));
    const el = document.createElement("div");
    Object.defineProperty(el, "scrollTop", { value: 0, writable: true, configurable: true });
    act(() => result.current.ref(el));
    expect(result.current.scrolled).toBe(false);
    el.scrollTop = 20;
    act(() => el.dispatchEvent(new Event("scroll")));
    expect(result.current.scrolled).toBe(true);
  });
});
