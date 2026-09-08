import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { Sidebar } from "./Sidebar";

describe("Sidebar", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("renders 4 nav items", () => {
    render(<Sidebar page="home" onSelect={() => {}} />);
    expect(screen.getByText("首页")).toBeInTheDocument();
    expect(screen.getByText("数据集")).toBeInTheDocument();
    expect(screen.getByText("搜索")).toBeInTheDocument();
    expect(screen.getByText("设置")).toBeInTheDocument();
  });

  it("calls onSelect when a nav item is clicked", () => {
    const onSelect = vi.fn();
    render(<Sidebar page="home" onSelect={onSelect} />);
    fireEvent.click(screen.getByText("数据集"));
    expect(onSelect).toHaveBeenCalledWith("datasets");
  });
});