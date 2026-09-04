import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import ThemeToggle from "./ThemeToggle";

describe("ThemeToggle", () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.className = "";
  });

  it("renders three options", () => {
    render(<ThemeToggle />);
    expect(screen.getByRole("button", { name: /浅色/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /暗色/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /跟随系统/ })).toBeInTheDocument();
  });

  it("marks the active option with aria-pressed", () => {
    localStorage.setItem("fan-files-theme", "dark");
    render(<ThemeToggle />);
    const darkBtn = screen.getByRole("button", { name: /暗色/ });
    expect(darkBtn.getAttribute("aria-pressed")).toBe("true");
  });

  it("clicking an option updates localStorage and html class", () => {
    render(<ThemeToggle />);
    fireEvent.click(screen.getByRole("button", { name: /暗色/ }));
    expect(localStorage.getItem("fan-files-theme")).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });
});
