import { describe, expect, it } from "vitest";
import { json, fakeApi } from "@/test/fakes";

const savedAnswer = { provider: "anthropic", keySet: true, keyHint: "0000", model: "example-model-a", usable: true, inUse: true };

describe("the agent provider's endpoints", () => {
  it("reads, saves with a PUT and forgets with a DELETE", async () => {
    const { server, api } = fakeApi();
    server.on("GET", "/api/agent/provider", () => json(200, { keySet: false, usable: false, inUse: false }));
    server.on("PUT", "/api/agent/provider", () => json(200, savedAnswer));
    server.on("DELETE", "/api/agent/provider", () => json(200, { keySet: false, usable: false, inUse: false }));

    expect(await api.agentProvider.provider()).toEqual({ keySet: false, usable: false, inUse: false });
    const request = { provider: "anthropic", apiKey: "sk-ant-example-key-0000", model: "example-model-a" };
    expect(await api.agentProvider.save(request)).toEqual(savedAnswer);
    expect(await api.agentProvider.forget()).toEqual({ keySet: false, usable: false, inUse: false });

    const [read, put, remove] = server.to("/api/agent/provider");
    expect(read!.method).toBe("GET");
    expect([put!.method, put!.headers["content-type"], put!.body]).toEqual(["PUT", "application/json", request]);
    expect([remove!.method, remove!.body]).toEqual(["DELETE", undefined]);
  });

  it("tests with a POST and throws the server's sentence", async () => {
    const { server, api } = fakeApi();
    const listed = { checked: true, models: [{ id: "example-model-a" }, { id: "example-model-b", name: "Example Model B" }] };
    server.on("POST", "/api/agent/provider/test", () => json(200, listed));
    expect(await api.agentProvider.test({ provider: "ollama" })).toEqual(listed);
    expect(server.to("/api/agent/provider/test")[0]!.body).toEqual({ provider: "ollama" });

    server.on("POST", "/api/agent/provider/test", () => json(422, { error: "The provider refused the key." }));
    await expect(api.agentProvider.test({ provider: "deepseek", apiKey: "example" })).rejects.toMatchObject({
      status: 422,
      message: "The provider refused the key.",
    });
  });
});
