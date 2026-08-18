import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import EngineBanner from "./EngineBanner";

describe("EngineBanner", () => {
  it("renders error and retry button", () => {
    render(<EngineBanner error="引擎未运行" onRetry={() => {}} />);
    expect(screen.getByText(/引擎未运行/)).toBeInTheDocument();
    expect(screen.getByText("重试")).toBeInTheDocument();
  });
  it("fires onRetry", () => {
    const onRetry = vi.fn();
    render(<EngineBanner error="x" onRetry={onRetry} />);
    fireEvent.click(screen.getByText("重试"));
    expect(onRetry).toHaveBeenCalled();
  });
});
