import { describe, it, expect, vi, afterEach } from "vitest";
import { fetchStats, fetchDatasets, searchDatasets, fetchDatasetDetail, fetchFiles, setApiBase, getApiBase } from "./api";

afterEach(() => {
  vi.unstubAllGlobals();
});

const statsBody = {
  data: {
    datasets_upper_bound: 3,
    assets_upper_bound: 4,
    files_upper_bound: 5,
    linked_files_upper_bound: 6,
    last_indexed_at: 1724000000,
    approximate: true,
  },
};

const summary = {
  id: 1,
  name: "Oryza_sativa_v1",
  type: "genome",
  species: "Oryza sativa",
  summary: "rice reference genome",
  path: "/data/orders/Oryza_sativa/v1",
  asset_count: 2,
  file_count: 42,
  updated_at: 1724000000,
};

describe("api client", () => {
  it("fetchStats calls /api/v1/stats and unwraps data", async () => {
    const m = vi.fn().mockResolvedValue({ ok: true, json: async () => statsBody });
    vi.stubGlobal("fetch", m);
    const stats = await fetchStats();
    expect(m).toHaveBeenCalledWith(expect.stringContaining("/api/v1/stats"));
    expect(stats.datasets_upper_bound).toBe(3);
  });

  it("searchDatasets encodes q", async () => {
    const m = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ data: [] }) });
    vi.stubGlobal("fetch", m);
    await searchDatasets("水稻 基因组");
    expect(m).toHaveBeenCalledWith(expect.stringContaining(encodeURIComponent("水稻 基因组")));
  });

  it("fetchDatasets builds cursor/limit params and keeps meta", async () => {
    const m = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        data: [summary],
        meta: { limit: 20, next_cursor: 41, has_more: true, sort: "id" },
      }),
    });
    vi.stubGlobal("fetch", m);
    const page = await fetchDatasets({ cursor: 42, limit: 20 });
    const url = m.mock.calls[0][0] as string;
    expect(url).toContain("cursor=42");
    expect(url).toContain("limit=20");
    expect(page.data).toHaveLength(1);
    expect(page.meta.next_cursor).toBe(41);
  });

  it("fetchDatasets omits undefined params", async () => {
    const m = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ data: [], meta: { limit: 50, next_cursor: null, has_more: false } }),
    });
    vi.stubGlobal("fetch", m);
    await fetchDatasets({});
    const url = m.mock.calls[0][0] as string;
    expect(url).not.toContain("undefined");
    expect(url).not.toContain("null");
    expect(url.endsWith("/api/v1/datasets")).toBe(true);
  });

  it("fetchDatasetDetail calls /api/v1/datasets/:id", async () => {
    const m = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        data: {
          id: 1,
          name: "Oryza_sativa_v1",
          type: "genome",
          species: "Oryza sativa",
          species_confidence: null,
          summary: null,
          path: "/data/orders/Oryza_sativa/v1",
          updated_at: 1724000000,
          assets: [{ id: 7, name: "assembly", type: "assembly", file_count: 3 }],
        },
      }),
    });
    vi.stubGlobal("fetch", m);
    const detail = await fetchDatasetDetail(1);
    expect(m).toHaveBeenCalledWith(expect.stringContaining("/api/v1/datasets/1"));
    expect(detail.assets[0].file_count).toBe(3);
  });

  it("fetchFiles calls /api/v1/datasets/:id/files with limit and optional cursor", async () => {
    const m = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        data: [
          {
            id: 101,
            asset_id: 7,
            name: "ref.fa",
            size: 123,
            role: null,
            mime_type: null,
            source_server: "srv",
            path: "/data/orders/rice/ref.fa",
          },
        ],
        meta: { limit: 50, next_cursor: null, has_more: false },
      }),
    });
    vi.stubGlobal("fetch", m);
    const page = await fetchFiles(1);
    let url = m.mock.calls[0][0] as string;
    expect(url).toContain("/api/v1/datasets/1/files");
    expect(url).toContain("limit=50");
    expect(url).not.toContain("cursor");
    await fetchFiles(1, 42);
    url = m.mock.calls[1][0] as string;
    expect(url).toContain("cursor=42");
    expect(page.data[0].name).toBe("ref.fa");
  });

  it("throws on non-ok responses", async () => {
    const m = vi.fn().mockResolvedValue({ ok: false, status: 503 });
    vi.stubGlobal("fetch", m);
    await expect(fetchStats()).rejects.toThrow("HTTP 503");
  });

  it("setApiBase switches the base port for subsequent calls", async () => {
    setApiBase(20217);
    const m = vi.fn().mockResolvedValue({ ok: true, json: async () => statsBody });
    vi.stubGlobal("fetch", m);
    await fetchStats();
    expect(m.mock.calls[0][0]).toBe("http://127.0.0.1:20217/api/v1/stats");
    setApiBase(17951); // 复位默认，避免影响其他测试
  });

  it("getApiBase reflects the last setApiBase port", () => {
    setApiBase(30284);
    expect(getApiBase()).toBe("http://127.0.0.1:30284");
    setApiBase(17951); // 复位默认，避免影响其他测试
    expect(getApiBase()).toBe("http://127.0.0.1:17951");
  });
});
