import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import DataTable from "./DataTable";
import type { DatasetSummary } from "../api";

const rows: DatasetSummary[] = [
  {
    id: 1,
    name: "Oryza_sativa_v1",
    type: "genome",
    species: "Oryza sativa",
    summary: null,
    path: "/data/orders/rice",
    asset_count: 2,
    file_count: 1234,
    updated_at: 1724000000,
  },
  {
    id: 2,
    name: "NoType",
    type: null,
    species: null,
    summary: null,
    path: null,
    asset_count: 0,
    file_count: 0,
    updated_at: 1724000001,
  },
];

describe("DataTable", () => {
  it("renders rows with type badge, species, file count and path", () => {
    render(<DataTable rows={rows} onSelect={() => {}} />);
    expect(screen.getByText("Oryza_sativa_v1")).toBeInTheDocument();
    expect(screen.getByText("Oryza sativa")).toBeInTheDocument();
    expect(screen.getByText("1,234")).toBeInTheDocument();
    expect(screen.getByText("/data/orders/rice")).toBeInTheDocument();
    const badge = screen.getByText("genome");
    expect(badge).toHaveClass("badge", "badge-genome");
    // null 类型 / null 路径都回退为 "—"（第二行就有两处）
    expect(screen.getAllByText("—").length).toBeGreaterThanOrEqual(2);
  });

  it("calls onSelect with the clicked row", () => {
    const onSelect = vi.fn();
    render(<DataTable rows={rows} onSelect={onSelect} />);
    fireEvent.click(screen.getByText("Oryza_sativa_v1"));
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledWith(rows[0]);
  });

  it("shows empty message when there are no rows", () => {
    render(<DataTable rows={[]} onSelect={() => {}} />);
    expect(screen.getByText("还没有数据集 — 去首页开始扫描")).toBeInTheDocument();
  });
});
