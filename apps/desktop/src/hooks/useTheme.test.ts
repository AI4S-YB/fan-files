import { describe, it, expect, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useTheme } from "./useTheme";

describe("useTheme", () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.className = "";
  });

  it("defaults to system when no stored value", () => {
    const { result } = renderHook(() => useTheme());
    expect(result.current[0]).toBe("system");
  });

  it("reads stored value", () => {
    localStorage.setItem("fan-files-theme", "dark");
    const { result } = renderHook(() => useTheme());
    expect(result.current[0]).toBe("dark");
  });

  it("ignores invalid stored value", () => {
    localStorage.setItem("fan-files-theme", "rainbow");
    const { result } = renderHook(() => useTheme());
    expect(result.current[0]).toBe("system");
  });

  it("setter updates state and applies class", () => {
    const { result } = renderHook(() => useTheme());
    act(() => result.current[1]("dark"));
    expect(result.current[0]).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(document.documentElement.classList.contains("light")).toBe(false);
  });

  it("switching from dark to light removes dark class", () => {
    const { result } = renderHook(() => useTheme());
    act(() => result.current[1]("dark"));
    act(() => result.current[1]("light"));
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    expect(document.documentElement.classList.contains("light")).toBe(true);
  });

  it("persists to localStorage", () => {
    const { result } = renderHook(() => useTheme());
    act(() => result.current[1]("light"));
    expect(localStorage.getItem("fan-files-theme")).toBe("light");
  });
});
