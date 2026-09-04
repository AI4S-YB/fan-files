import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { AppShell } from "./AppShell";

describe("AppShell", () => {
  it("renders title in header", () => {
    render(<AppShell title="首页"><div>content</div></AppShell>);
    expect(screen.getByRole("heading", { name: "首页" })).toBeInTheDocument();
  });

  it("renders children in main", () => {
    render(<AppShell title="X"><span data-testid="kid">kid</span></AppShell>);
    expect(screen.getByTestId("kid")).toBeInTheDocument();
  });

  it("renders actions in header", () => {
    render(<AppShell title="X" actions={<button>go</button>}>
      <div>content</div>
    </AppShell>);
    expect(screen.getByRole("button", { name: "go" })).toBeInTheDocument();
  });

  it("header has glass-header class", () => {
    const { container } = render(<AppShell title="X"><div /></AppShell>);
    expect(container.querySelector("header.glass-header")).toBeInTheDocument();
  });
});
