import { beforeEach, describe, expect, it, vi } from "vitest";

import { registerAppProtocol } from "./protocol";

const mocks = vi.hoisted(() => ({
  handle: vi.fn(),
  fetch: vi.fn(),
}));

vi.mock("electron", () => ({
  net: {
    fetch: mocks.fetch,
  },
  protocol: {
    registerSchemesAsPrivileged: vi.fn(),
    handle: mocks.handle,
  },
}));

describe("registerAppProtocol", () => {
  beforeEach(() => {
    mocks.handle.mockReset();
    mocks.fetch.mockReset();
    mocks.fetch.mockResolvedValue(new Response("ok"));
  });

  it("serves renderer files from app bundle URLs", async () => {
    registerAppProtocol("main_window");

    const handler = protocolHandler();
    await handler(new Request("app://bundle/"));
    expect(mocks.fetch).toHaveBeenLastCalledWith(
      expect.stringMatching(/^file:.*\/renderer\/main_window\/index\.html$/),
    );

    await handler(new Request("app://bundle/assets/app.js"));
    expect(mocks.fetch).toHaveBeenLastCalledWith(
      expect.stringMatching(/^file:.*\/renderer\/main_window\/assets\/app\.js$/),
    );
  });

  it("rejects non-bundle hosts", async () => {
    registerAppProtocol("main_window");

    const response = await protocolHandler()(new Request("app://other/"));

    expect(response.status).toBe(404);
    expect(mocks.fetch).not.toHaveBeenCalled();
  });

  it("rejects path traversal outside the renderer root", async () => {
    registerAppProtocol("main_window");

    const response = await protocolHandler()(new Request("app://bundle/%252e%252e/main.js"));

    expect(response.status).toBe(403);
    expect(mocks.fetch).not.toHaveBeenCalled();
  });
});

function protocolHandler() {
  const handler = mocks.handle.mock.calls[0]?.[1] as (
    request: Request,
  ) => Promise<Response> | Response;

  expect(mocks.handle.mock.calls[0]?.[0]).toBe("app");
  return handler;
}
