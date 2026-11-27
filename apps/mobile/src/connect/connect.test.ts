import { describe, expect, it } from "vitest";
import { connectVectors } from "@/test/vectors";
import { decodeBase64Url, encodeBase32Lower, encodeBase64Url } from "./encoding";
import { decodeUtf8Lossy, parseForm } from "./form";
import { formatIpv6, isNormalHost, isNormalSocketAddress, parseIpv6 } from "./hosts";
import { checkMachine, machineIdError, machineIdOf, statement } from "./machineId";
import { parsePairingLink } from "./pairingLink";
import { checkProbeAnswer, linkRouteKind, probeBinds, readProbeAnswer } from "./probe";
import { isPublicKey, verifyStrict } from "./signature";

interface Encodings {
  nonces: { valid: { hex: string; text: string }[]; invalid: { text: string; error: string }[] };
  machineIds: { valid: string[]; invalid: { text: string; error: string }[] };
  publicKeys: { invalid: { text: string; error: string }[] };
  signatures: { valid: { hex: string; text: string }[]; invalid: { text: string; error: string }[] };
}

interface Keys {
  keys: { name: string; publicKey: string; publicKeyHex: string; machineIdInput: string; machineIdHash: string; machineId: string }[];
}

interface Links {
  valid: { link: string; id: string; code: string; routes: string[] }[];
  invalid: { link: string; error: string }[];
}

interface Connect {
  messages: { pairAnswer: { machine: unknown }; pairAnswerLegacy: Record<string, unknown> };
  withUnknownFields: { pairAnswer: { machine: unknown } };
  unreadable: { pairAnswerWrongKey: { message: { machine: unknown }; refusedBy: string } };
}

const hex = (bytes: Uint8Array) => Buffer.from(bytes).toString("hex");
const fromHex = (text: string) => new Uint8Array(Buffer.from(text, "hex"));

describe("the encodings, against the shared vectors", () => {
  const vectors = connectVectors<Encodings>("encodings.json");

  it("reads base64url of 32 and 64 bytes in its one spelling", () => {
    for (const { hex: bytes, text } of vectors.nonces.valid) {
      const decoded = decodeBase64Url(text, 32);
      expect(decoded.ok && hex(decoded.bytes)).toBe(bytes);
      expect(encodeBase64Url(fromHex(bytes))).toBe(text);
    }
    for (const { hex: bytes, text } of vectors.signatures.valid) {
      const decoded = decodeBase64Url(text, 64);
      expect(decoded.ok && hex(decoded.bytes)).toBe(bytes);
    }
  });

  it("refuses padding, other alphabets and set trailing bits", () => {
    for (const { text, error } of vectors.nonces.invalid) expect(decodeBase64Url(text, 32), text).toEqual({ ok: false, error });
    for (const { text, error } of vectors.signatures.invalid) expect(decodeBase64Url(text, 64), text).toEqual({ ok: false, error });
    // The public keys' encoding errors; the rest of their cases are curve checks, which a key's id doesn't need.
    for (const { text, error } of vectors.publicKeys.invalid.filter((v) => v.error === "length"))
      expect(decodeBase64Url(text, 32), text).toEqual({ ok: false, error });
  });

  it("reads machine ids only as 32 characters of lowercase base32", () => {
    for (const id of vectors.machineIds.valid) expect(machineIdError(id)).toBeNull();
    for (const { text, error } of vectors.machineIds.invalid) expect(machineIdError(text), text).toBe(error);
    // As the Rust reader orders its checks: upper case before the length, and the length in UTF-8 bytes.
    expect(machineIdError("3j6rajbbrd2yrrvdkoq2mouz5aqxwh5Ya")).toBe("symbol");
    expect(machineIdError("ctspvuq66v5mc4d5fpjvm2khgwzlc4é")).toBe("symbol");
    expect(machineIdError("3j6rajbbrd2yrrvdkoq2mouz5aqxwh5é")).toBe("length");
  });
});

describe("the machine id", () => {
  const { keys } = connectVectors<Keys>("keys.json");

  it("is the first 160 bits of the hash of the key's statement, in lowercase base32", () => {
    for (const key of keys) {
      const publicKey = fromHex(key.publicKeyHex);
      expect(hex(statement("emberhut machine id v1", [publicKey]))).toBe(key.machineIdInput);
      expect(encodeBase32Lower(fromHex(key.machineIdHash).subarray(0, 20))).toBe(key.machineId);
      expect(machineIdOf(publicKey), key.name).toBe(key.machineId);
      const decoded = decodeBase64Url(key.publicKey, 32);
      expect(decoded.ok && hex(decoded.bytes)).toBe(key.publicKeyHex);
    }
  });
});

describe("a pairing answer's machine", () => {
  const vectors = connectVectors<Connect>("connect.json");
  const machine = "3j6rajbbrd2yrrvdkoq2mouz5aqxwh5c";
  const other = "qoasjogo65qh2cy62xmytganhyz74fla";

  it("proves its id with its key, unknown fields and all", () => {
    expect(checkMachine(vectors.messages.pairAnswer.machine, machine)).toMatchObject({ ok: true, machine: { id: machine } });
    expect(checkMachine(vectors.withUnknownFields.pairAnswer.machine, machine)).toMatchObject({ ok: true });
    expect(checkMachine(vectors.messages.pairAnswer.machine, null)).toMatchObject({ ok: true, machine: { id: machine } });
  });

  it("is refused when its key gives another id", () => {
    const wrong = vectors.unreadable.pairAnswerWrongKey;
    expect(wrong.refusedBy).toBe("wrongKey");
    expect(checkMachine(wrong.message.machine, null)).toEqual({ ok: false, reason: "wrongKey" });
    expect(checkMachine(wrong.message.machine, other)).toEqual({ ok: false, reason: "wrongKey" });
    expect(checkMachine({ id: machine, publicKey: "not a key" }, null)).toEqual({ ok: false, reason: "wrongKey" });
    expect(checkMachine("a machine", null)).toEqual({ ok: false, reason: "wrongKey" });
  });

  it("must be the machine the phone meant", () => {
    expect(checkMachine(vectors.messages.pairAnswer.machine, other)).toEqual({ ok: false, reason: "otherMachine" });
  });

  it("may be missing only from a first, typed pairing: stripping it can't downgrade a link's check", () => {
    expect(vectors.messages.pairAnswerLegacy).not.toHaveProperty("machine");
    expect(checkMachine(vectors.messages.pairAnswerLegacy.machine, null)).toEqual({ ok: true, machine: null });
    expect(checkMachine(vectors.messages.pairAnswerLegacy.machine, machine)).toEqual({ ok: false, reason: "missing" });
    expect(checkMachine(null, machine)).toEqual({ ok: false, reason: "missing" });
  });
});

describe("the pairing link, against the shared vectors", () => {
  const vectors = connectVectors<Links>("pairing_links.json");

  it("reads every valid link to its id, code and routes", () => {
    for (const { link, id, code, routes } of vectors.valid)
      expect(parsePairingLink(link), link).toEqual({ ok: true, link: { id, code, routes } });
  });

  it("refuses every invalid one for the vector's reason", () => {
    for (const { link, error } of vectors.invalid) {
      const parsed = parsePairingLink(link);
      expect(parsed.ok, link).toBe(false);
      if (!parsed.ok) expect(parsed.error.kind, link).toBe(error);
    }
  });

  it("names the key that is missing or repeated", () => {
    const id = "3j6rajbbrd2yrrvdkoq2mouz5aqxwh5c";
    expect(parsePairingLink(`https://emberhut.com/pair#code=A`)).toEqual({ ok: false, error: { kind: "missing", key: "id" } });
    expect(parsePairingLink(`https://emberhut.com/pair#id=${id}`)).toEqual({ ok: false, error: { kind: "missing", key: "code" } });
    expect(parsePairingLink(`https://emberhut.com/pair#id=${id}&code=A&code=B`)).toEqual({
      ok: false,
      error: { kind: "repeated", key: "code" },
    });
  });

  it("counts its length in characters, as the Rust reader does", () => {
    const id = "3j6rajbbrd2yrrvdkoq2mouz5aqxwh5c";
    const head = `https://emberhut.com/pair#id=${id}&code=A&x=`;
    // 2,000 characters, some of them taking two UTF-16 units.
    const fits = head + "😀".repeat(2_000 - head.length);
    expect(parsePairingLink(fits).ok).toBe(true);
    expect(parsePairingLink(`${fits}a`)).toEqual({ ok: false, error: { kind: "tooLong" } });
  });
});

describe("form encoding", () => {
  it("reads pairs as the URL Standard does: + is a space, bad escapes stay, empty pieces are skipped", () => {
    expect(parseForm("a=1&&b=x+y&c&d=%41%2&e=%zz&=f")).toEqual([
      ["a", "1"],
      ["b", "x y"],
      ["c", ""],
      ["d", "A%2"],
      ["e", "%zz"],
      ["", "f"],
    ]);
    expect(parseForm("r=https%3A%2F%2Fexample.com%2Fa%3Fb%3Dc%26d%23e&s=%C3%A9")).toEqual([
      ["r", "https://example.com/a?b=c&d#e"],
      ["s", "é"],
    ]);
  });

  it("puts one replacement character for each broken UTF-8 sequence, as from_utf8_lossy does", () => {
    const lossy = (bytes: number[]) => decodeUtf8Lossy(bytes);
    expect(lossy([0xff])).toBe("�");
    expect(lossy([0xe2, 0x82])).toBe("�");
    expect(lossy([0xe2, 0x82, 0x41])).toBe("�A");
    expect(lossy([0xc0, 0xaf])).toBe("��");
    expect(lossy([0xed, 0xa0, 0x80])).toBe("���");
    expect(lossy([0xf0, 0x9f, 0x98, 0x80])).toBe("😀");
    for (const bytes of [
      [0xff, 0x41],
      [0xe2, 0x82],
      [0xf4, 0x90, 0x80, 0x80],
      [0xc3, 0xa9, 0xe2, 0x82, 0xac],
    ])
      expect(lossy(bytes)).toBe(new TextDecoder().decode(new Uint8Array(bytes)));
  });
});

interface Signatures {
  statement: { bytes: string };
  cases: { name: string; publicKey: string; signature: string; accept: boolean; keyError?: string }[];
  crossKind: { name: string; publicKey: string; signature: string; accept: boolean }[];
  speccheck: { case: number; message: string; publicKey: string; signature: string; accept: boolean }[];
}

describe("signatures, by the contract's one rule", () => {
  const vectors = connectVectors<Signatures>("signatures.json");
  const message = fromHex(vectors.statement.bytes);
  const bytes = (text: string, size: number) => {
    const decoded = decodeBase64Url(text, size);
    if (!decoded.ok) throw new Error(`not base64url: ${text}`);
    return decoded.bytes;
  };

  it("accepts and refuses each case as the vectors say, keys that don't read included", () => {
    for (const c of [...vectors.cases, ...vectors.crossKind]) {
      const key = bytes(c.publicKey, 32);
      expect(verifyStrict(key, message, bytes(c.signature, 64)), c.name).toBe(c.accept);
      if ("keyError" in c && c.keyError !== undefined) expect(isPublicKey(key), c.name).toBe(false);
    }
  });

  it("refuses all twelve cases of Taming the many EdDSAs that the rule refuses", () => {
    for (const c of vectors.speccheck)
      expect(verifyStrict(fromHex(c.publicKey), fromHex(c.message), fromHex(c.signature)), `case ${c.case}`).toBe(c.accept);
  });

  it("reads no public key that is weak, of mixed order or not a point", () => {
    const { publicKeys } = connectVectors<Encodings>("encodings.json");
    for (const { text, error } of publicKeys.invalid) {
      const decoded = decodeBase64Url(text, 32);
      if (error === "length") expect(decoded.ok).toBe(false);
      else expect(decoded.ok && isPublicKey(decoded.bytes), text).toBe(false);
    }
    expect(isPublicKey(bytes("iJ6VrUsjsH5tpp2N75DjqJTSWs5mm3l979UsEGknM3g", 32))).toBe(true);
  });
});

interface Hosts {
  normalize: { normal: string }[];
  invalid: { input: string }[];
  parse: { text: string; normal?: string; error?: string }[];
  whatwgDifferences: { cases: { input: string }[] };
}

interface Sockets {
  normalize: { normal: string }[];
  parse: { text: string; normal?: string; error?: string }[];
}

describe("hosts and socket addresses in normal form", () => {
  it("reads a host only when it is already in normal form", () => {
    const hosts = connectVectors<Hosts>("hosts.json");
    for (const { text, error } of hosts.parse) expect(isNormalHost(text), text).toBe(error === undefined);
    for (const { normal } of hosts.normalize) expect(isNormalHost(normal), normal).toBe(true);
    for (const { input } of [...hosts.invalid, ...hosts.whatwgDifferences.cases]) expect(isNormalHost(input), input).toBe(false);
  });

  it("reads a socket address only when it is already in normal form", () => {
    const sockets = connectVectors<Sockets>("socket_addresses.json");
    for (const { text, error } of sockets.parse) expect(isNormalSocketAddress(text), text).toBe(error === undefined);
    for (const { normal } of sockets.normalize) expect(isNormalSocketAddress(normal), normal).toBe(true);
  });

  it("writes IPv6 addresses in RFC 5952 form", () => {
    const form = (text: string) => formatIpv6(parseIpv6(text)!);
    expect(form("2001:0DB8:0:0::1")).toBe("2001:db8::1");
    expect(form("2001:db8:0:0:1:0:0:1")).toBe("2001:db8::1:0:0:1");
    expect(form("2001:db8:0:1:1:1:1:1")).toBe("2001:db8:0:1:1:1:1:1");
    expect(form("::")).toBe("::");
    expect(form("::1.2.3.4")).toBe("::102:304");
    expect(form("1:2:3:4:5:6:1.2.3.4")).toBe("1:2:3:4:5:6:102:304");
    for (const bad of ["1:2:3:4:5:6:7", "1::2::3", ":1::", "1:2:3:4:5:6:7:8:9", "12345::", "fe80::1%en0", "1.2.3.4"])
      expect(parseIpv6(bad), bad).toBeNull();
  });
});

interface Probes {
  probes: {
    description: string;
    query: { nonce: string; kind: string };
    expected: string;
    answer: unknown;
    valid?: boolean;
    error?: string;
    refusedBy?: string;
  }[];
  answers: Record<string, { kind: string; answer: unknown }>;
  bindings: { answer: string; route: { kind: string; url: string }; matches: boolean; why: string }[];
}

describe("the probe, against the shared vectors", () => {
  const vectors = connectVectors<Probes>("probes.json");

  it("reads, checks and refuses each answer as the vectors say", () => {
    for (const probe of vectors.probes) {
      const read = readProbeAnswer(probe.answer);
      if (probe.error === "decode") {
        expect(read, probe.description).toEqual({ ok: false, refusedBy: probe.refusedBy });
        continue;
      }
      expect(read.ok, probe.description).toBe(true);
      if (!read.ok) continue;
      const nonce = decodeBase64Url(probe.query.nonce, 32);
      if (!nonce.ok) throw new Error("bad nonce");
      const checked = checkProbeAnswer(read.answer, { nonce: nonce.bytes, kind: probe.query.kind, expected: probe.expected });
      expect(checked, probe.description).toEqual(probe.valid ? { ok: true } : { ok: false, error: probe.error });
    }
  });

  it("binds an answer to the route it was sent on as the vectors say", () => {
    for (const binding of vectors.bindings) {
      const { kind, answer } = vectors.answers[binding.answer]!;
      const read = readProbeAnswer(answer);
      expect(read.ok).toBe(true);
      if (read.ok) expect(probeBinds(read.answer, kind, binding.route), `${binding.route.url}: ${binding.why}`).toBe(binding.matches);
    }
  });

  it("probes a link's route as the kind its URL gives", () => {
    expect(linkRouteKind("http://127.0.0.1:18990")).toBe("loopback");
    expect(linkRouteKind("http://[::1]:18990")).toBe("loopback");
    expect(linkRouteKind("http://192.168.1.10:18990")).toBe("lan");
    expect(linkRouteKind("http://[2001:db8::10]:18990")).toBe("lan");
    expect(linkRouteKind("https://emberhut.example")).toBe("public");
    expect(linkRouteKind("wss://relay.example/v1/sealed/x")).toBeNull();
  });
});
