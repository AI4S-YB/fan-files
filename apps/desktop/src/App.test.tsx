import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import App from "./App";

describe("App shell", () => {
  it("renders sidebar with four entries", () => {
    render(<App />);
    for (const label of ["首页", "数据集", "搜索", "设置"]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
  });
  it("switches page on sidebar click", () => {
    render(<App />);
    fireEvent.click(screen.getByText("数据集"));
    expect(screen.getByText("数据集页面（占位）")).toBeInTheDocument();
  });
});
