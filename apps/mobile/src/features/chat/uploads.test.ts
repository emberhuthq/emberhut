import { describe, expect, it } from "vitest";
import {
  maxUploadBytes,
  pickFailure,
  readyFiles,
  uploadFailure,
  uploadKind,
  uploadName,
  uploadRefusal,
  uploadsReducer,
  type Upload,
} from "./uploads";

const now = Date.parse("2026-11-24T10:00:00Z");

describe("uploadName", () => {
  it("keeps the picker's name, without slashes", () => {
    expect(uploadName({ uri: "file:///c/1.jpg", name: "IMG 0001.jpg" }, now)).toBe("IMG 0001.jpg");
    expect(uploadName({ uri: "file:///c/x", name: "a/b.pdf" }, now)).toBe("a-b.pdf");
  });

  it("makes one up from the type, else the address", () => {
    expect(uploadName({ uri: "file:///c/x", mimeType: "image/jpeg" }, now)).toBe("photo-1795514400.jpg");
    expect(uploadName({ uri: "file:///c/clip.MOV?x=1", mimeType: "video/unknown" }, now)).toBe("video-1795514400.mov");
    expect(uploadName({ uri: "content://media/42" }, now)).toBe("file-1795514400.bin");
  });
});

describe("failures", () => {
  it("keeps the server's reason for a failed upload", () => {
    expect(uploadFailure(new Error("Too large for this server"))).toBe("Too large for this server");
    expect(uploadFailure(new Error(""))).toBeUndefined();
    expect(uploadFailure("gone")).toBeUndefined();
  });

  it("says which picker failed, and why when it's known", () => {
    expect(pickFailure("photos", new Error("No access to the library"))).toBe("Couldn't open the photos: No access to the library");
    expect(pickFailure("files", undefined)).toBe("Couldn't open the files.");
  });
});

describe("uploadKind", () => {
  it("goes by the type, then the name", () => {
    expect(uploadKind({ uri: "x", mimeType: "image/png" })).toBe("image");
    expect(uploadKind({ uri: "x", mimeType: "video/mp4" })).toBe("video");
    expect(uploadKind({ uri: "x", name: "clip.mp4" })).toBe("video");
    expect(uploadKind({ uri: "x", name: "report.html", mimeType: "text/html" })).toBe("file");
  });
});

describe("uploads", () => {
  const up = (id: string): Upload => ({ id, name: `${id}.png`, kind: "image", uri: `file:///${id}`, status: "sending" });
  const file = (id: string) => ({ name: `${id}.png`, url: `/uploads/${id}.png`, kind: "image" as const, path: `/srv/${id}.png` });

  it("goes with the message once every file is up, the failed ones left out", () => {
    let s = uploadsReducer([], { type: "add", uploads: [up("a"), up("b"), up("c")] });
    s = uploadsReducer(s, { type: "sent", id: "a", file: file("a") });
    expect(readyFiles(s)).toEqual({ files: [file("a")], sending: true });
    s = uploadsReducer(s, { type: "failed", id: "b", error: "Over 500 MB: too big to send" });
    expect(s.find((u) => u.id === "b")?.error).toBe("Over 500 MB: too big to send");
    s = uploadsReducer(s, { type: "remove", id: "c" });
    expect(readyFiles(s)).toEqual({ files: [file("a")], sending: false });
    expect(uploadsReducer(s, { type: "clear" })).toEqual([]);
  });
});

describe("uploadRefusal", () => {
  it("refuses a file over 500 MB before it goes up", () => {
    expect(uploadRefusal(maxUploadBytes)).toBeNull();
    expect(uploadRefusal(maxUploadBytes + 1)).toBe("Over 500 MB: too big to send");
  });
});
