"""Writes the shared connect test vectors (see README.md beside this file).

It is a second implementation, independent of the Rust one: Ed25519 signing through
`cryptography` (OpenSSL), the verification rule and the crafted points in plain Python below,
and hashlib, base64 and urllib for the rest. Run it as `python3 -I gen.py <out dir>`.
"""

import base64
import hashlib
import json
import os
import struct
import sys
import unicodedata
from urllib.parse import quote

import cryptography
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

CRYPTOGRAPHY = "41.0.7"
if cryptography.__version__ != CRYPTOGRAPHY:
    sys.exit(f"needs cryptography {CRYPTOGRAPHY}, found {cryptography.__version__}")

OUT = sys.argv[1]
TIME = 1795561073000
MAX_TIME = 2**53 - 1
DOMAIN = "relay.example"
ACCOUNT = "user_2example0account"
NAME = "Main computer"


def b64(b):
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode()


def b32(b):
    return base64.b32encode(b).rstrip(b"=").decode().lower()


def statement(prefix, fields):
    out = prefix.encode("ascii") + b"\0"
    for f in fields:
        if isinstance(f, str):
            f = f.encode("utf-8")
        elif isinstance(f, int):
            f = struct.pack(">Q", f)
        out += struct.pack(">I", len(f)) + f
    return out


def write(name, value):
    with open(os.path.join(OUT, name), "w", encoding="utf-8") as f:
        json.dump(value, f, indent=2, ensure_ascii=False)
        f.write("\n")


def with_unknown_fields(value):
    """The value with a field no version knows added to every object in it."""
    if isinstance(value, dict):
        out = {k: with_unknown_fields(v) for k, v in value.items()}
        out["fromALaterVersion"] = {"anything": [1, "two", None]}
        return out
    if isinstance(value, list):
        return [with_unknown_fields(v) for v in value]
    return value


# --- Ed25519 arithmetic (RFC 8032), for the verification rule and crafted points -----------
P = 2**255 - 19
L = 2**252 + 27742317777372353535851937790883648493
D = (-121665 * pow(121666, P - 2, P)) % P
SQRT_M1 = pow(2, (P - 1) // 4, P)
IDENTITY = (0, 1, 1, 0)


def inv(x):
    return pow(x, P - 2, P)


def add(p, q):
    a = (p[1] - p[0]) * (q[1] - q[0]) % P
    b = (p[1] + p[0]) * (q[1] + q[0]) % P
    c = 2 * p[3] * q[3] * D % P
    d = 2 * p[2] * q[2] % P
    e, f, g, h = b - a, d - c, d + c, b + a
    return (e * f % P, g * h % P, f * g % P, e * h % P)


def neg(p):
    return ((P - p[0]) % P, p[1], p[2], (P - p[3]) % P)


def mul(s, p):
    q = IDENTITY
    while s > 0:
        if s & 1:
            q = add(q, p)
        p = add(p, p)
        s >>= 1
    return q


def equal(p, q):
    return (p[0] * q[2] - q[0] * p[2]) % P == 0 and (p[1] * q[2] - q[1] * p[2]) % P == 0


def affine(x, y):
    return (x % P, y % P, 1, x * y % P)


def recover_x(y, sign):
    if y >= P:
        return None
    x2 = (y * y - 1) * inv(D * y * y + 1) % P
    if x2 == 0:
        return None if sign else 0
    x = pow(x2, (P + 3) // 8, P)
    if (x * x - x2) % P:
        x = x * SQRT_M1 % P
    if (x * x - x2) % P:
        return None
    if (x & 1) != sign:
        x = P - x
    return x


def encode(p):
    zi = inv(p[2])
    x, y = p[0] * zi % P, p[1] * zi % P
    return (y | ((x & 1) << 255)).to_bytes(32, "little")


def decode(b):
    """A canonical encoding's point, or None."""
    n = int.from_bytes(b, "little")
    y, sign = n & ((1 << 255) - 1), n >> 255
    x = recover_x(y, sign)
    return None if x is None else affine(x, y)


B = affine(recover_x(4 * inv(5) % P, 0), 4 * inv(5) % P)


def small_order(p):
    return equal(mul(8, p), IDENTITY)


def torsion_free(p):
    return equal(mul(L, p), IDENTITY)


def h512(*parts):
    return int.from_bytes(hashlib.sha512(b"".join(parts)).digest(), "little")


def verify(a_bytes, msg, sig):
    """docs/api.md's rule: s < L; A and R canonical, in the prime-order subgroup, not of small
    order; [s]B = R + [k]A without the cofactor."""
    a = decode(a_bytes)
    if a is None or small_order(a) or not torsion_free(a):
        return False
    r_bytes, s = sig[:32], int.from_bytes(sig[32:], "little")
    r = decode(r_bytes)
    if r is None or small_order(r) or s >= L:
        return False
    k = h512(r_bytes, a_bytes, msg) % L
    return encode(add(mul(s, B), neg(mul(k, a)))) == r_bytes


def cofactored(a_bytes, msg, sig):
    """The lax cofactored check, on any encodings it can read: [8][s]B = [8]R + [8][k]A."""
    def lax(b):
        n = int.from_bytes(b, "little")
        y, sign = (n & ((1 << 255) - 1)) % P, n >> 255
        x = recover_x(y, sign)
        return None if x is None else affine(x, y)
    a, r = lax(a_bytes), lax(sig[:32])
    s = int.from_bytes(sig[32:], "little")
    k = h512(sig[:32], a_bytes, msg) % L
    return equal(mul(8, mul(s, B)), add(mul(8, r), mul(8, mul(k, a))))


def point_of_order(order):
    for y in range(2, 1000):
        x = recover_x(y, 0)
        if x is None:
            continue
        t = mul(L, affine(x, y))
        if not equal(mul(order // 2, t), IDENTITY) and equal(mul(order, t), IDENTITY):
            return t
    raise RuntimeError("no point")


T8 = point_of_order(8)
T4 = affine(SQRT_M1, 0)
assert small_order(T4) and not equal(mul(2, T4), IDENTITY)


class Key:
    def __init__(self, name, seed, how):
        self.name, self.seed, self.how = name, seed, how
        self.sk = Ed25519PrivateKey.from_private_bytes(seed)
        self.pk = self.sk.public_key().public_bytes(
            serialization.Encoding.Raw, serialization.PublicFormat.Raw)
        h = hashlib.sha512(seed).digest()
        a = int.from_bytes(h[:32], "little")
        self.scalar = (a & ((1 << 254) - 8)) | (1 << 254)
        self.point = mul(self.scalar, B)
        assert encode(self.point) == self.pk
        self.id_input = statement("emberhut machine id v1", [self.pk])
        self.id_hash = hashlib.sha256(self.id_input).digest()
        self.id = b32(self.id_hash[:20])

    def sign(self, msg):
        sig = self.sk.sign(msg)
        assert verify(self.pk, msg, sig)
        return sig

    def json(self):
        return {"name": self.name, "seed": self.seed.hex(), "seedFrom": self.how,
                "publicKey": b64(self.pk), "publicKeyHex": self.pk.hex(),
                "machineIdInput": self.id_input.hex(), "machineIdHash": self.id_hash.hex(),
                "machineId": self.id}


def derived(name, label):
    return Key(name, hashlib.sha256(label.encode()).digest(),
               f'SHA-256 of the ASCII text "{label}"')


RFC = Key("rfc8032-test-1", bytes.fromhex(
    "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60"),
    "RFC 8032, section 7.1, test 1")
assert RFC.pk.hex() == "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
MACHINE = derived("machine", "emberhut test vector key: machine")
OTHER = derived("other-machine", "emberhut test vector key: other machine")
DEVICE = derived("device", "emberhut test vector key: device")
KEYS = [RFC, MACHINE, OTHER, DEVICE]

NONCE = hashlib.sha256(b"emberhut test vector nonce: probe").digest()
NONCE2 = hashlib.sha256(b"emberhut test vector nonce: another probe").digest()
HELLO_NONCE = hashlib.sha256(b"emberhut test vector nonce: relay hello").digest()
RELAY_HOST = f"{MACHINE.id}.{DOMAIN}"

write("keys.json", {
    "description": "Test-vector keys from fixed seeds (never use them for anything else). The seed is the RFC 8032 secret key; the machine id is the first 20 bytes of machineIdHash = SHA-256(machineIdInput), in lowercase base32 without padding.",
    "keys": [k.json() for k in KEYS],
})

# --- Encodings ------------------------------------------------------------------------------
identity_bytes = encode(IDENTITY)
non_canonical_identity = (P + 1).to_bytes(32, "little")
mixed_machine = encode(add(MACHINE.point, T8))
write("encodings.json", {
    "description": "Binary values in JSON (nonces, keys, signatures) are base64url without padding (RFC 4648, section 5); a decoder refuses padding, other alphabets and non-zero trailing bits. Machine ids are 32 characters of lowercase base32. Times are whole milliseconds from 0 to 2^53 - 1. Errors: length, symbol, trailingBits; for public keys also notAPoint, weak, notTorsionFree; for times outOfRange.",
    "nonces": {
        "valid": [
            {"hex": "00" * 32, "text": b64(bytes(32))},
            {"hex": "ff" * 32, "text": b64(b"\xff" * 32)},
            {"hex": NONCE.hex(), "text": b64(NONCE)},
        ],
        "invalid": [
            {"text": b64(NONCE) + "=", "error": "length", "why": "padding"},
            {"text": b64(NONCE)[:-1], "error": "length", "why": "42 characters"},
            {"text": b64(NONCE)[:-1] + "B", "error": "trailingBits", "why": "a set bit past the 256th"},
            {"text": "+" + b64(NONCE)[1:], "error": "symbol", "why": "the standard alphabet's +"},
            {"text": "/" + b64(NONCE)[1:], "error": "symbol", "why": "the standard alphabet's /"},
            {"text": " " + b64(NONCE)[1:], "error": "symbol", "why": "whitespace"},
        ],
    },
    "machineIds": {
        "valid": [k.id for k in KEYS],
        "invalid": [
            {"text": MACHINE.id.upper(), "error": "symbol", "why": "upper case"},
            {"text": MACHINE.id[:-1], "error": "length", "why": "31 characters"},
            {"text": MACHINE.id + "a", "error": "length", "why": "33 characters"},
            {"text": MACHINE.id[:-1] + "1", "error": "symbol", "why": "1 is not in the alphabet"},
            {"text": MACHINE.id[:-1] + "=", "error": "symbol", "why": "padding"},
        ],
    },
    "publicKeys": {
        "invalid": [
            {"text": b64(identity_bytes), "error": "weak", "why": "the identity point has small order"},
            {"text": b64(encode(T8)), "error": "weak", "why": "a point of order 8"},
            {"text": b64(mixed_machine), "error": "notTorsionFree", "why": "the machine's key plus a point of order 8"},
            {"text": b64(non_canonical_identity), "error": "notAPoint", "why": "y = p + 1, a non-canonical encoding"},
            {"text": b64(MACHINE.pk) + "=", "error": "length", "why": "padding"},
        ],
    },
    "signatures": {
        "valid": [{"hex": "fb" * 64, "text": b64(b"\xfb" * 64)}],
        "invalid": [
            {"text": b64(bytes(64))[:-1], "error": "length", "why": "85 characters"},
            {"text": b64(bytes(64))[:-1] + "B", "error": "trailingBits", "why": "a set bit past the 512th"},
        ],
    },
    "times": {
        "valid": [0, TIME, MAX_TIME],
        "invalid": [{"millis": MAX_TIME + 1, "error": "outOfRange"}, {"millis": 2**64 - 1, "error": "outOfRange"}],
    },
})

# --- Hosts ----------------------------------------------------------------------------------
label63 = "a" * 63
write("hosts.json", {
    "description": "A host's normal form: a DNS name (UTS 46 with the STD3 rules but _ allowed, hyphens checked and DNS lengths verified: lowercase letters, digits, hyphens and underscores, labels of 1 to 63 characters, at most 253, no trailing dot) or an IPv4 address in dotted decimal or an IPv6 address in RFC 5952 form in brackets (not IPv4-mapped, no zone), and the port dropped when it is the scheme's default. `parse` reads text already in normal form (the scheme isn't known, so a default port stays). Errors: empty, port, ipv6, name, numericName, notNormal.",
    "normalize": [
        {"input": "Example.COM", "scheme": "https", "normal": "example.com"},
        {"input": "example.com:443", "scheme": "https", "normal": "example.com"},
        {"input": "example.com:443", "scheme": "http", "normal": "example.com:443"},
        {"input": "example.com:80", "scheme": "http", "normal": "example.com"},
        {"input": "example.com:080", "scheme": "https", "normal": "example.com:80"},
        {"input": "Bücher.example:8443", "scheme": "https", "normal": "xn--bcher-kva.example:8443"},
        {"input": "faß.de", "scheme": "https", "normal": "xn--fa-hia.de"},
        {"input": "ｅｘａｍｐｌｅ.com", "scheme": "https", "normal": "example.com"},
        {"input": "192.0.2.1:18990", "scheme": "http", "normal": "192.0.2.1:18990"},
        {"input": "１９２.０.２.１", "scheme": "http", "normal": "192.0.2.1"},
        {"input": "127.0.0.1:18990", "scheme": "http", "normal": "127.0.0.1:18990"},
        {"input": "[2001:0DB8:0:0::1]:443", "scheme": "https", "normal": "[2001:db8::1]"},
        {"input": "[2001:db8:0:0:1:0:0:1]:18990", "scheme": "http", "normal": "[2001:db8::1:0:0:1]:18990"},
        {"input": "[2001:db8:0:1:1:1:1:1]", "scheme": "http", "normal": "[2001:db8:0:1:1:1:1:1]"},
        {"input": "[::1]:18990", "scheme": "http", "normal": "[::1]:18990"},
        {"input": RELAY_HOST, "scheme": "https", "normal": RELAY_HOST},
        {"input": "My_Box.local:18990", "scheme": "http", "normal": "my_box.local:18990", "why": "_ is allowed in a host name"},
        {"input": "emberhut_server_1", "scheme": "http", "normal": "emberhut_server_1", "why": "a container's name"},
        {"input": label63 + ".example", "scheme": "https", "normal": label63 + ".example", "why": "a label of 63"},
        {"input": ".".join([label63] * 4)[2:], "scheme": "https", "normal": ".".join([label63] * 4)[2:], "why": "253 characters"},
    ],
    "invalid": [
        {"input": "", "scheme": "http", "error": "empty"},
        {"input": ":80", "scheme": "http", "error": "empty"},
        {"input": "example.com:65536", "scheme": "http", "error": "port"},
        {"input": "example.com:+80", "scheme": "http", "error": "port"},
        {"input": "[::1]80", "scheme": "http", "error": "port"},
        {"input": "::1", "scheme": "http", "error": "ipv6"},
        {"input": "[fe80::1%25en0]", "scheme": "http", "error": "ipv6"},
        {"input": "[192.0.2.1]", "scheme": "http", "error": "ipv6"},
        {"input": "exa mple.com", "scheme": "http", "error": "name"},
        {"input": "a/b", "scheme": "http", "error": "name"},
        {"input": "a" + label63 + ".example", "scheme": "https", "error": "name", "why": "a label of 64"},
        {"input": ".".join([label63] * 4)[1:], "scheme": "https", "error": "name", "why": "254 characters"},
        {"input": "1.2.3.4.5", "scheme": "http", "error": "numericName"},
    ],
    "whatwgDifferences": {
        "description": "Every input where these rules differ from the WHATWG URL Standard's host parsing (what `new URL()` does), with what that parser makes of it.",
        "cases": [
            {"input": "127.1", "scheme": "http", "whatwg": "127.0.0.1", "error": "numericName", "why": "IPv4 in a short form"},
            {"input": "0x7f.1", "scheme": "http", "whatwg": "127.0.0.1", "error": "numericName", "why": "IPv4 in hexadecimal"},
            {"input": "192.0.2.010", "scheme": "http", "whatwg": "192.0.2.8", "error": "numericName", "why": "IPv4 with an octal part"},
            {"input": "[::ffff:192.0.2.1]", "scheme": "http", "whatwg": "[::ffff:c000:201]", "error": "ipv6", "why": "an IPv4-mapped IPv6 address"},
            {"input": "[::ffff:c000:201]", "scheme": "http", "whatwg": "[::ffff:c000:201]", "error": "ipv6", "why": "an IPv4-mapped IPv6 address, in hexadecimal"},
            {"input": "re;lay.example", "scheme": "https", "whatwg": "re;lay.example", "error": "name", "why": ";"},
            {"input": "a,b.example", "scheme": "https", "whatwg": "a,b.example", "error": "name", "why": ","},
            {"input": "x=y.example", "scheme": "https", "whatwg": "x=y.example", "error": "name", "why": "="},
            {"input": '"q".example', "scheme": "https", "whatwg": '"q".example', "error": "name", "why": "a double quote"},
            {"input": "a*b.example", "scheme": "https", "whatwg": "a*b.example", "error": "name", "why": "*"},
            {"input": "-bad-.example", "scheme": "https", "whatwg": "-bad-.example", "error": "name", "why": "a hyphen at a label's ends"},
            {"input": "ab--cd.example", "scheme": "https", "whatwg": "ab--cd.example", "error": "name", "why": "hyphens in the third and fourth places"},
            {"input": "a" + label63 + ".example", "scheme": "https", "whatwg": "a" + label63 + ".example", "error": "name", "why": "a label over 63 characters"},
            {"input": "example.com.", "scheme": "https", "whatwg": "example.com.", "error": "name", "why": "a trailing dot"},
            {"input": "ex%61mple.com", "scheme": "https", "whatwg": "example.com", "error": "name", "why": "percent-encoding"},
            {"input": "user@example.com", "scheme": "https", "whatwg": "example.com", "error": "name", "why": "user information: a host is not an authority"},
            {"input": "example.com:0", "scheme": "https", "whatwg": "example.com:0", "error": "port", "why": "port 0"},
            {"input": "example.com:", "scheme": "https", "whatwg": "example.com", "error": "port", "why": "an empty port"},
        ],
    },
    "parse": [
        {"text": "example.com", "normal": "example.com"},
        {"text": "example.com:443", "normal": "example.com:443"},
        {"text": "[2001:db8::1]:18990", "normal": "[2001:db8::1]:18990"},
        {"text": "Example.com", "error": "notNormal", "normalForm": "example.com"},
        {"text": "[2001:DB8::1]", "error": "notNormal", "normalForm": "[2001:db8::1]"},
        {"text": "example.com:018990", "error": "notNormal", "normalForm": "example.com:18990"},
        {"text": "Bücher.example", "error": "notNormal", "normalForm": "xn--bcher-kva.example"},
        {"text": "::1", "error": "ipv6"},
    ],
    "relayDomains": {
        "description": "A relay domain is a host name in that normal form with no port, under strict STD3 (no _), and at most 220 characters, so <machine id>.<relay domain> fits DNS's 253. Errors: host (not a host name), notAName (a port or an address), tooLong.",
        "valid": [
            {"input": "Relay.Example", "normal": "relay.example"},
            {"input": "relay.example", "normal": "relay.example"},
            {"input": label63 + ".example", "normal": label63 + ".example", "why": "a label of 63"},
            {"input": ".".join(["b" * 20] + [label63] * 3 + ["example"]), "normal": ".".join(["b" * 20] + [label63] * 3 + ["example"]), "why": "220 characters"},
        ],
        "invalid": [
            {"input": "relay.example:443", "error": "notAName"},
            {"input": "192.0.2.1", "error": "notAName"},
            {"input": "[2001:db8::1]", "error": "notAName"},
            {"input": "relay.example.", "error": "host"},
            {"input": "a..example", "error": "host"},
            {"input": "re;lay.example", "error": "host"},
            {"input": "a_b.example", "error": "host", "why": "_ is allowed in hosts, not in relay domains"},
            {"input": "a,b.example", "error": "host"},
            {"input": "x=y.example", "error": "host"},
            {"input": '"q".example', "error": "host"},
            {"input": "a*b.example", "error": "host"},
            {"input": "-bad-.example", "error": "host"},
            {"input": "a" + label63 + ".example", "error": "host", "why": "a label of 64"},
            {"input": ".".join(["b" * 21] + [label63] * 3 + ["example"]), "error": "tooLong", "why": "221 characters"},
            {"input": "", "error": "host"},
        ],
    },
})
for case in json.load(open(os.path.join(OUT, "hosts.json")))["relayDomains"]["valid"]:
    assert len(case["normal"]) <= 220

write("socket_addresses.json", {
    "description": "A local socket address's normal form: a.b.c.d:port or [v6]:port, the IPv6 address in RFC 5952 form, no zone id, and an IPv4-mapped IPv6 address as the IPv4 address. `parse` reads text that must already be in normal form. Errors: invalid, notNormal.",
    "normalize": [
        {"input": "192.0.2.1:18990", "normal": "192.0.2.1:18990"},
        {"input": "[::ffff:192.0.2.1]:18990", "normal": "192.0.2.1:18990"},
        {"input": "[2001:0DB8:0000::0001]:18990", "normal": "[2001:db8::1]:18990"},
        {"input": "[2001:db8:0:0:1:0:0:1]:80", "normal": "[2001:db8::1:0:0:1]:80"},
        {"input": "[::1]:18990", "normal": "[::1]:18990"},
    ],
    "parse": [
        {"text": "192.0.2.1:18990", "normal": "192.0.2.1:18990"},
        {"text": "[2001:db8::1]:18990", "normal": "[2001:db8::1]:18990"},
        {"text": "[::ffff:192.0.2.1]:18990", "error": "notNormal", "normalForm": "192.0.2.1:18990"},
        {"text": "[2001:DB8::1]:18990", "error": "notNormal", "normalForm": "[2001:db8::1]:18990"},
        {"text": "[2001:db8:0::1]:18990", "error": "notNormal", "normalForm": "[2001:db8::1]:18990"},
        {"text": "192.0.2.1", "error": "invalid"},
        {"text": "example.com:80", "error": "invalid"},
        {"text": "", "error": "invalid"},
    ],
})

# --- Names, kinds and account ids -------------------------------------------------------------
WHITE_SPACE = set(map(chr, list(range(0x9, 0xE)) + [0x20, 0x85, 0xA0, 0x1680] + list(range(0x2000, 0x200B)) + [0x2028, 0x2029, 0x202F, 0x205F, 0x3000]))


def trim(text):
    start, end = 0, len(text)
    while start < end and text[start] in WHITE_SPACE:
        start += 1
    while end > start and text[end - 1] in WHITE_SPACE:
        end -= 1
    return text[start:end]


BIDI = {0x061C, 0x200E, 0x200F} | set(range(0x202A, 0x202F)) | set(range(0x2066, 0x206A))


def refused(c):
    return unicodedata.category(c) == "Cc" or ord(c) in BIDI


def cut(text):
    name = trim(trim(text)[:60])
    if name and any(map(refused, name)):
        return None
    return name


def parse_name(text):
    if not text:
        return "empty"
    if trim(text) != text:
        return "notTrimmed"
    if len(text) > 60:
        return "tooLong"
    if any(map(refused, text)):
        return "control"
    return None


EMOJI = "\U0001F5A5"
cut_cases = [
    ("  Main computer \n", "a name with white space around it"),
    ("é" * 61, "61 characters"),
    (EMOJI * 61, "61 astral characters: 60 kept, 120 UTF-16 units"),
    ("a" * 59 + " x", "white space where the cut falls is trimmed too"),
    ("﻿Main﻿", "U+FEFF is not white space"),
    ("\u0085Main\u0085", "U+0085 is white space"),
    ("​Main", "U+200B is not white space"),
    ("Main computer " + EMOJI, "an emoji kept"),
    ("\tMain\n", "controls at the ends are white space, trimmed"),
]
control_cases = ["Main\tbox", "Main\u0007", "\u202eMain", "a\u2066b", "a\u061cb", "a\u200eb", "a\u200fb", "a\u0085b", "a\u0000b"]
names = {"cut": [], "parse": []}
for text, why in cut_cases:
    assert cut(text)
    names["cut"].append({"input": text, "name": cut(text), "why": why})
for text in control_cases:
    assert cut(text) is None and parse_name(text) == "control"
    names["cut"].append({"input": text, "error": "control"})
for text in [" 　\u0085 ", ""]:
    assert cut(text) == ""
    names["cut"].append({"input": text, "error": "empty"})
names["parse"] = [
    {"text": "Main computer", "name": "Main computer"},
    {"text": EMOJI * 60, "name": EMOJI * 60, "why": "60 astral characters"},
    {"text": "﻿Main﻿", "name": "﻿Main﻿"},
    {"text": "é" * 61, "error": "tooLong"},
    {"text": EMOJI * 61, "error": "tooLong"},
    {"text": " Main", "error": "notTrimmed"},
    {"text": "Main\u0085", "error": "notTrimmed"},
    {"text": "Main　", "error": "notTrimmed"},
    {"text": "", "error": "empty"},
] + [{"text": text, "error": "control"} for text in control_cases]
for case in names["parse"]:
    assert parse_name(case["text"]) == case.get("error"), case
write("names.json", {
    "description": "Machine names: 1 to 60 Unicode scalar values with no control character (Cc) or bidirectional control (U+061C, U+200E, U+200F, U+202A-U+202E, U+2066-U+2069) and no Unicode White_Space at either end (U+0009-U+000D, U+0020, U+0085, U+00A0, U+1680, U+2000-U+200A, U+2028, U+2029, U+202F, U+205F, U+3000; not U+FEFF, which JavaScript's trim() removes, and including U+0085, which it keeps). A sender cuts (trims, keeps the first 60, trims the end again) before signing; a receiver parses and refuses rather than cut. Probe kinds are 1 to 32 characters from a-z, 0-9 and -. Account ids are 1 to 255 bytes of visible ASCII. Errors: empty, tooLong, notTrimmed, control; kind; length, character.",
    "machineNames": names,
    "probeKinds": {
        "valid": ["loopback", "lan", "vpn", "public", "ssh", "relay", "a-later-kind", "k" * 32],
        "invalid": ["", "Lan", "a b", "lan\n", "ü", "k" * 33, "a_b"],
    },
    "accountIds": {
        "valid": [ACCOUNT, "a" * 255, "!~"],
        "invalid": [{"text": "", "error": "length"}, {"text": "a" * 256, "error": "length"},
                    {"text": "user x", "error": "character"}, {"text": "user_é", "error": "character"}],
    },
})

# --- Statements -------------------------------------------------------------------------------
statements = []


def add_statement(name, prefix, inputs, fields, signer):
    msg = statement(prefix, fields)
    statements.append({"name": name, "prefix": prefix, "inputs": inputs, "signer": signer.name,
                       "bytes": msg.hex(), "signature": b64(signer.sign(msg))})
    return msg


def probe_inputs(nonce, mid, kind, via, local, host):
    return {"nonce": b64(nonce), "machineId": mid, "kind": kind, "via": via, "local": local, "host": host}


for name, args in [
    ("probe, direct on the local network", (NONCE, MACHINE.id, "lan", "direct", "192.0.2.10:18990", "192.0.2.10:18990")),
    ("probe, relayed", (NONCE, MACHINE.id, "relay", "relayed", "", RELAY_HOST)),
    ("probe, direct over IPv6", (NONCE2, MACHINE.id, "vpn", "direct", "[2001:db8::10]:18990", "[2001:db8::10]:18990")),
]:
    add_statement(name, "emberhut probe v1", probe_inputs(*args), list(args), MACHINE)
add_statement("relay auth", "emberhut relay auth v1",
              {"relayDomain": DOMAIN, "nonce": b64(HELLO_NONCE), "name": NAME, "time": TIME},
              [DOMAIN, HELLO_NONCE, NAME, TIME], MACHINE)
link_msg = add_statement("link", "emberhut link v1",
                         {"account": ACCOUNT, "relayDomain": DOMAIN, "name": NAME, "time": TIME},
                         [ACCOUNT, DOMAIN, NAME, TIME], MACHINE)
unlink_msg = add_statement("unlink", "emberhut unlink v1",
                           {"machineId": MACHINE.id, "relayDomain": DOMAIN, "time": TIME},
                           [MACHINE.id, DOMAIN, TIME], MACHINE)
add_statement("link, by the RFC 8032 key, with a name beyond ASCII", "emberhut link v1",
              {"account": "user_x", "relayDomain": DOMAIN, "name": "Café " + EMOJI, "time": 0},
              ["user_x", DOMAIN, "Café " + EMOJI, 0], RFC)
write("statements.json", {
    "description": "statement(prefix, fields): the ASCII prefix and a 0x00 byte, then each field as a 4-byte big-endian length and its bytes. A time is 8 bytes of big-endian milliseconds; a key or nonce its raw bytes; text its UTF-8 bytes in normal form (the machine id as its 32 characters). Signatures are Ed25519 (RFC 8032) over the bytes, by the named key of keys.json.",
    "statements": statements,
})

# --- Signatures: the verification rule -------------------------------------------------------
def scalar(label):
    return h512(label.encode()) % L


def sig_bytes(r_bytes, s):
    return r_bytes + s.to_bytes(32, "little")


unlink_inputs = {"machineId": MACHINE.id, "relayDomain": DOMAIN, "time": TIME}
sig_cases = []


def sig_case(name, a_bytes, sig, accept, key_error=None, why=""):
    assert verify(a_bytes, unlink_msg, sig) == accept, name
    case = {"name": name, "publicKey": b64(a_bytes), "signature": b64(sig), "accept": accept, "why": why}
    if key_error:
        case["keyError"] = key_error
    sig_cases.append(case)


valid = MACHINE.sign(unlink_msg)
sig_case("valid", MACHINE.pk, valid, True, why="the baseline")
s = int.from_bytes(valid[32:], "little")
sig_case("s + L", MACHINE.pk, sig_bytes(valid[:32], s + L), False,
         why="s is not below L; the same signature with s reduced verifies")

a = MACHINE.scalar
r_bytes = encode(T4)
k = h512(r_bytes, MACHINE.pk, unlink_msg) % L
sig = sig_bytes(r_bytes, k * a % L)
assert cofactored(MACHINE.pk, unlink_msg, sig)
sig_case("small-order R", MACHINE.pk, sig, False, why="R has order 4; a cofactored check accepts it")

r_bytes = non_canonical_identity
k = h512(r_bytes, MACHINE.pk, unlink_msg) % L
sig = sig_bytes(r_bytes, k * a % L)
assert cofactored(MACHINE.pk, unlink_msg, sig)
sig_case("non-canonical R", MACHINE.pk, sig, False,
         why="R is the identity spelled y = p + 1; a check that reduces y accepts it")

r = scalar("emberhut test vector: torsioned R")
r_point = add(mul(r, B), T8)
r_bytes = encode(r_point)
k = h512(r_bytes, MACHINE.pk, unlink_msg) % L
sig = sig_bytes(r_bytes, (r + k * a) % L)
assert cofactored(MACHINE.pk, unlink_msg, sig)
sig_case("R with a small-order component", MACHINE.pk, sig, False,
         why="R = [r]B + T, T of order 8; a cofactored check accepts it")

mixed = add(MACHINE.point, T8)
mixed_bytes = encode(mixed)
for i in range(1000):
    r = scalar(f"emberhut test vector: mixed A {i}")
    r_bytes = encode(mul(r, B))
    k = h512(r_bytes, mixed_bytes, unlink_msg) % L
    if k % 8 == 0:
        break
sig = sig_bytes(r_bytes, (r + k * a) % L)
assert equal(mul((r + k * a) % L, B), add(mul(r, B), mul(k, mixed)))
sig_case("mixed-order A", mixed_bytes, sig, False, "notTorsionFree",
         why="A = the machine's key + T, T of order 8, and k a multiple of 8, so the cofactorless equation holds; only the subgroup check refuses it")

r = scalar("emberhut test vector: non-canonical A")
r_bytes = encode(mul(r, B))
sig_case("non-canonical A", non_canonical_identity, sig_bytes(r_bytes, r), False, "notAPoint",
         why="A is the identity spelled y = p + 1; any statement would verify")
sig_case("small-order A", identity_bytes, sig_bytes(r_bytes, r), False, "weak",
         why="A is the identity; any statement would verify")
t8_bytes = encode(T8)
for i in range(1000):
    r = scalar(f"emberhut test vector: order-8 A {i}")
    r_bytes = encode(mul(r, B))
    if h512(r_bytes, t8_bytes, unlink_msg) % L % 8 == 0:
        break
sig_case("order-8 A", t8_bytes, sig_bytes(r_bytes, r), False, "weak",
         why="A has order 8 and k is a multiple of 8, so the cofactorless equation holds")

# A signature of one kind presented as another.
link_sig = MACHINE.sign(link_msg)
cross = [{"name": "a link signature presented as an unlink", "signature": b64(link_sig),
          "publicKey": b64(MACHINE.pk), "accept": False}]
assert not verify(MACHINE.pk, unlink_msg, link_sig)

SPECCHECK = [
    ("8c93255d71dcab10e8f379c26200f3c7bd5f09d9bc3068d3ef4edeb4853022b6", "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac03fa", "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a0000000000000000000000000000000000000000000000000000000000000000"),
    ("9bd9f44f4dcc75bd531b56b2cd280b0bb38fc1cd6d1230e14861d861de092e79", "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac03fa", "f7badec5b8abeaf699583992219b7b223f1df3fbbea919844e3f7c554a43dd43a5bb704786be79fc476f91d3f3f89b03984d8068dcf1bb7dfc6637b45450ac04"),
    ("aebf3f2601a0c8c5d39cc7d8911642f740b78168218da8471772b35f9d35b9ab", "f7badec5b8abeaf699583992219b7b223f1df3fbbea919844e3f7c554a43dd43", "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac03fa8c4bd45aecaca5b24fb97bc10ac27ac8751a7dfe1baff8b953ec9f5833ca260e"),
    ("9bd9f44f4dcc75bd531b56b2cd280b0bb38fc1cd6d1230e14861d861de092e79", "cdb267ce40c5cd45306fa5d2f29731459387dbf9eb933b7bd5aed9a765b88d4d", "9046a64750444938de19f227bb80485e92b83fdb4b6506c160484c016cc1852f87909e14428a7a1d62e9f22f3d3ad7802db02eb2e688b6c52fcd6648a98bd009"),
    ("e47d62c63f830dc7a6851a0b1f33ae4bb2f507fb6cffec4011eaccd55b53f56c", "cdb267ce40c5cd45306fa5d2f29731459387dbf9eb933b7bd5aed9a765b88d4d", "160a1cb0dc9c0258cd0a7d23e94d8fa878bcb1925f2c64246b2dee1796bed5125ec6bc982a269b723e0668e540911a9a6a58921d6925e434ab10aa7940551a09"),
    ("e47d62c63f830dc7a6851a0b1f33ae4bb2f507fb6cffec4011eaccd55b53f56c", "cdb267ce40c5cd45306fa5d2f29731459387dbf9eb933b7bd5aed9a765b88d4d", "21122a84e0b5fca4052f5b1235c80a537878b38f3142356b2c2384ebad4668b7e40bc836dac0f71076f9abe3a53f9c03c1ceeeddb658d0030494ace586687405"),
    ("85e241a07d148b41e47d62c63f830dc7a6851a0b1f33ae4bb2f507fb6cffec40", "442aad9f089ad9e14647b1ef9099a1ff4798d78589e66f28eca69c11f582a623", "e96f66be976d82e60150baecff9906684aebb1ef181f67a7189ac78ea23b6c0e547f7690a0e2ddcd04d87dbc3490dc19b3b3052f7ff0538cb68afb369ba3a514"),
    ("85e241a07d148b41e47d62c63f830dc7a6851a0b1f33ae4bb2f507fb6cffec40", "442aad9f089ad9e14647b1ef9099a1ff4798d78589e66f28eca69c11f582a623", "8ce5b96c8f26d0ab6c47958c9e68b937104cd36e13c33566acd2fe8d38aa19427e71f98a473474f2f13f06f97c20d58cc3f54b8bd0d272f42b695dd7e89a8c22"),
    ("9bedc267423725d473888631ebf45988bad3db83851ee85c85e241a07d148b41", "f7badec5b8abeaf699583992219b7b223f1df3fbbea919844e3f7c554a43dd43", "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff03be9678ac102edcd92b0210bb34d7428d12ffc5df5f37e359941266a4e35f0f"),
    ("9bedc267423725d473888631ebf45988bad3db83851ee85c85e241a07d148b41", "f7badec5b8abeaf699583992219b7b223f1df3fbbea919844e3f7c554a43dd43", "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffca8c5b64cd208982aa38d4936621a4775aa233aa0505711d8fdcfdaa943d4908"),
    ("e96b7021eb39c1a163b6da4e3093dcd3f21387da4cc4572be588fafae23c155b", "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff", "a9d55260f765261eb9b84e106f665e00b867287a761990d7135963ee0a7d59dca5bb704786be79fc476f91d3f3f89b03984d8068dcf1bb7dfc6637b45450ac04"),
    ("39a591f5321bbe07fd5a23dc2f39d025d74526615746727ceefd6e82ae65c06f", "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff", "a9d55260f765261eb9b84e106f665e00b867287a761990d7135963ee0a7d59dca5bb704786be79fc476f91d3f3f89b03984d8068dcf1bb7dfc6637b45450ac04"),
]
speccheck = [{"case": i, "message": m, "publicKey": pk, "signature": sg,
              "accept": verify(bytes.fromhex(pk), bytes.fromhex(m), bytes.fromhex(sg))}
             for i, (m, pk, sg) in enumerate(SPECCHECK)]
write("signatures.json", {
    "description": "The one verification rule both implementations follow (docs/api.md): s < L; A and R canonical encodings of points in the prime-order subgroup, neither of small order; then [s]B = R + [k]A without the cofactor, k = SHA-512(R || A || M) mod L. A public key that breaks the rule for A doesn't read (keyError). `cases` sign the unlink statement of `statement`; `crossKind` presents another statement's signature for it; `speccheck` are the twelve cases of \"Taming the many EdDSAs\" (Chalkias, Garillot and Nikolaenko, 2020; github.com/novifinancial/ed25519-speccheck, Apache-2.0), over raw messages in hex.",
    "statement": {"prefix": "emberhut unlink v1", "inputs": unlink_inputs, "bytes": unlink_msg.hex()},
    "cases": sig_cases,
    "crossKind": cross,
    "speccheck": speccheck,
})

# --- Probe answers -----------------------------------------------------------------------------
def probe_answer(signer, key, mid, nonce, kind, via, local, host, sign_nonce=None):
    msg = statement("emberhut probe v1", [sign_nonce or nonce, mid, kind, via, local, host])
    return {"id": mid, "publicKey": b64(key.pk), "via": via, "local": local, "host": host,
            "signature": b64(signer.sk.sign(msg))}


probes = []


def probe(description, kind, answer, nonce=NONCE, expected=MACHINE, error=None, refused_by=None):
    assert (error == "decode") == (refused_by is not None)
    probes.append({"description": description, "query": {"nonce": b64(nonce), "kind": kind},
                   "expected": expected.id, "answer": answer,
                   **({"error": error} if error else {"valid": True}),
                   **({"refusedBy": refused_by} if refused_by else {})})


LAN = ("192.0.2.10:18990", "192.0.2.10:18990")
answers = {
    "lan": probe_answer(MACHINE, MACHINE, MACHINE.id, NONCE, "lan", "direct", *LAN),
    "relay": probe_answer(MACHINE, MACHINE, MACHINE.id, NONCE, "relay", "relayed", "", RELAY_HOST),
    "public": probe_answer(MACHINE, MACHINE, MACHINE.id, NONCE, "public", "direct", "127.0.0.1:18990", "emberhut.example"),
    "proxyDefaultHost": probe_answer(MACHINE, MACHINE, MACHINE.id, NONCE, "public", "direct", "127.0.0.1:18990", "127.0.0.1:18990"),
    "loopback": probe_answer(MACHINE, MACHINE, MACHINE.id, NONCE, "loopback", "direct", "127.0.0.1:18990", "127.0.0.1:18990"),
    "vpn": probe_answer(MACHINE, MACHINE, MACHINE.id, NONCE, "vpn", "direct", "[2001:db8::10]:18990", "[2001:db8::10]:18990"),
    "vpnName": probe_answer(MACHINE, MACHINE, MACHINE.id, NONCE, "vpn", "direct", "127.0.0.1:18990", "box.vpn.example"),
    "ssh": probe_answer(MACHINE, MACHINE, MACHINE.id, NONCE, "ssh", "direct", "127.0.0.1:18990", "127.0.0.1:49152"),
    "sshOverIpv6": probe_answer(MACHINE, MACHINE, MACHINE.id, NONCE, "ssh", "direct", "[::1]:18990", "127.0.0.1:49152"),
    "sshNotLoopback": probe_answer(MACHINE, MACHINE, MACHINE.id, NONCE, "ssh", "direct", "192.0.2.10:18990", "127.0.0.1:49152"),
    "sshRelayed": probe_answer(MACHINE, MACHINE, MACHINE.id, NONCE, "ssh", "relayed", "", "127.0.0.1:49152"),
}
kinds = {"lan": "lan", "relay": "relay", "public": "public", "proxyDefaultHost": "public",
         "loopback": "loopback", "vpn": "vpn", "vpnName": "vpn", "ssh": "ssh", "sshOverIpv6": "ssh",
         "sshNotLoopback": "ssh", "sshRelayed": "ssh"}
for name, answer in answers.items():
    probe(f"a valid answer: {name}", kinds[name], answer)
probe("a valid answer with a kind this version doesn't know", "carrier-pigeon",
      probe_answer(MACHINE, MACHINE, MACHINE.id, NONCE, "carrier-pigeon", "direct", "127.0.0.1:18990", "127.0.0.1:18990"))
probe("another machine answered", "lan",
      probe_answer(OTHER, OTHER, OTHER.id, NONCE, "lan", "direct", *LAN), error="otherMachine")
probe("signed by another key than the one it shows", "lan",
      probe_answer(OTHER, MACHINE, MACHINE.id, NONCE, "lan", "direct", *LAN), error="signature")
probe("signed for an earlier probe's nonce", "lan",
      probe_answer(MACHINE, MACHINE, MACHINE.id, NONCE, "lan", "direct", *LAN, sign_nonce=NONCE2), error="signature")
probe("signed for another route kind", "relay",
      probe_answer(MACHINE, MACHINE, MACHINE.id, NONCE, "lan", "direct", *LAN), error="signature")
probe("the key of another machine id", "lan",
      probe_answer(MACHINE, MACHINE, OTHER.id, NONCE, "lan", "direct", *LAN), error="decode", refused_by="wrongKey")
probe("direct, without its local address", "lan",
      probe_answer(MACHINE, MACHINE, MACHINE.id, NONCE, "lan", "direct", "", LAN[1]), error="decode", refused_by="reached")
probe("relayed, with a local address", "relay",
      probe_answer(MACHINE, MACHINE, MACHINE.id, NONCE, "relay", "relayed", LAN[0], RELAY_HOST), error="decode", refused_by="reached")
probe("a host not in normal form", "public",
      probe_answer(MACHINE, MACHINE, MACHINE.id, NONCE, "public", "direct", "127.0.0.1:18990", "Emberhut.example"), error="decode", refused_by="host")
probe("a local address not in normal form", "lan",
      probe_answer(MACHINE, MACHINE, MACHINE.id, NONCE, "lan", "direct", "[::ffff:192.0.2.10]:18990", LAN[1]), error="decode", refused_by="local")
weak = dict(answers["lan"], publicKey=b64(identity_bytes))
probe("a weak public key", "lan", weak, error="decode", refused_by="publicKey")
probe("an unknown via", "lan", dict(answers["lan"], via="teleported"), error="decode", refused_by="via")

bindings = []


def binding(answer, kind, url, matches, why):
    bindings.append({"answer": answer, "route": {"kind": kind, "url": url, "label": None, "up": True},
                     "matches": matches, "why": why})


binding("lan", "lan", "http://192.0.2.10:18990", True, "the address and port it was reached on")
binding("lan", "lan", "http://192.0.2.10:18990/", True, "a path doesn't matter")
binding("lan", "lan", "http://192.0.2.11:18990", False, "another address: a forward from a remembered one")
binding("lan", "lan", "http://192.0.2.10:18991", False, "another port")
binding("lan", "lan", "http://192.0.2.10", False, "the default port 80 is another port")
binding("lan", "lan", "http://mybox.example:18990", False, "an address route needs an address")
binding("relay", "relay", RELAY_HOST, False, "not a URL: no scheme")
binding("lan", "public", "https://192.0.2.10:18990", False, "the answer was for a lan probe")
binding("loopback", "loopback", "http://127.0.0.1:18990", True, "loopback")
binding("relay", "relay", f"https://{RELAY_HOST}", True, "relayed, for the relay's name")
binding("relay", "relay", f"https://{RELAY_HOST}:443/", True, "the default port")
binding("relay", "relay", f"https://{MACHINE.id}.other.example", False, "a relay URL under another domain")
binding("relay", "relay", f"http://{RELAY_HOST}", False, "a name route must be https")
binding("public", "public", "https://emberhut.example", True, "the public name, reached directly")
binding("public", "public", "https://Emberhut.Example:443/path", True, "compared in normal form")
binding("public", "public", "https://other.example", False, "another name")
binding("public", "public", "https://emberhut.example:8443", False, "another port")
binding("proxyDefaultHost", "public", "https://emberhut.example", False, "a proxy that sends its own Host")
binding("vpn", "vpn", "http://[2001:db8::10]:18990", True, "a VPN address")
binding("vpn", "vpn", "http://[2001:db8::11]:18990", False, "another VPN address")
binding("vpnName", "vpn", "https://box.vpn.example", True, "a VPN's HTTPS name")
binding("vpnName", "vpn", "http://box.vpn.example", False, "a VPN name must be https")
binding("ssh", "ssh", "ssh://mybox.example", False, "an ssh route is checked against its forward, not its URL")

ssh_bindings = []


def ssh_binding(answer, local_port, machine_port, matches, why):
    ssh_bindings.append({"answer": answer, "forward": {"localPort": local_port, "machinePort": machine_port},
                         "matches": matches, "why": why})


ssh_binding("ssh", 49152, 18990, True, "loopback on the forward's target port, for the forward's own port")
ssh_binding("sshOverIpv6", 49152, 18990, True, "IPv6 loopback on the machine is loopback too")
ssh_binding("ssh", 49152, 18991, False, "another target port")
ssh_binding("ssh", 49153, 18990, False, "another forward: the host is another local port")
ssh_binding("sshNotLoopback", 49152, 18990, False, "not on the machine's loopback")
ssh_binding("sshRelayed", 49152, 18990, False, "relayed")
ssh_binding("loopback", 18990, 18990, False, "a loopback probe, not an ssh one")
binding("lan", "carrier-pigeon", "http://192.0.2.10:18990", False, "a kind this version doesn't know")
write("probes.json", {
    "description": "GET /api/connect/probe: the query the client sent, the machine it expected, and the answer it got. An answer that reads has a key with its id and a via that agrees with local; a valid one is from the expected machine and has a signature over emberhut probe v1 with the query's nonce and kind. Errors: decode, otherMachine, signature. `bindings` match a valid answer (by its name in `answers`) against the route the client probed, and `sshBindings` an ssh answer against the local forward (ssh -L 127.0.0.1:<localPort>:127.0.0.1:<machinePort>). A decode case's `refusedBy` names what refused it: wrongKey, reached, host, local, publicKey, via. `kinds` are what a probe's kind may be; a server answers 400 to any other.",
    "probes": probes,
    "answers": {name: {"kind": kinds[name], "answer": a} for name, a in answers.items()},
    "nonce": b64(NONCE),
    "bindings": bindings,
    "sshBindings": ssh_bindings,
})

# --- Pairing links -----------------------------------------------------------------------------
def enc(v):
    return quote(v, safe="*-._").replace("%20", "+")


def link(pairs):
    return "https://emberhut.com/pair#" + "&".join(f"{k}={enc(v)}" for k, v in pairs)


routes = ["http://192.0.2.10:18990", "http://[2001:db8::10]:18990", f"https://{RELAY_HOST}"]
valid_links = [
    {"link": link([("id", MACHINE.id), ("code", "ABCD-EFGH")] + [("route", r) for r in routes]), "id": MACHINE.id, "code": "ABCD-EFGH", "routes": routes, "canonical": True},
    {"link": link([("id", MACHINE.id), ("code", "ABCD-EFGH")]), "id": MACHINE.id, "code": "ABCD-EFGH", "routes": [], "canonical": True},
    {"link": link([("id", MACHINE.id), ("code", "ABCD-EFGH"), ("route", "https://example.com/a?b=c&d#e")]), "id": MACHINE.id, "code": "ABCD-EFGH", "routes": ["https://example.com/a?b=c&d#e"], "canonical": True},
    {"link": "https://emberhut.com/pair#v=2&route=r1&id=" + MACHINE.id + "&other&code=abcd+efgh&route=r2", "id": MACHINE.id, "code": "abcd efgh", "routes": ["r1", "r2"], "canonical": False},
    {"link": "https://emberhut.com/pair#id=" + MACHINE.id + "&code=ABCD%2DEFGH&route=http:%2F%2F192.0.2.10:18990", "id": MACHINE.id, "code": "ABCD-EFGH", "routes": ["http://192.0.2.10:18990"], "canonical": False},
]
base = "https://emberhut.com/pair#id=" + MACHINE.id + "&code=ABCD-EFGH"
invalid_links = [
    {"link": "https://emberhut.com/pair?id=" + MACHINE.id + "&code=ABCD-EFGH", "error": "notAPairingLink"},
    {"link": "https://example.com/pair#id=" + MACHINE.id + "&code=ABCD-EFGH", "error": "notAPairingLink"},
    {"link": "http://emberhut.com/pair#id=" + MACHINE.id + "&code=ABCD-EFGH", "error": "notAPairingLink"},
    {"link": "https://emberhut.com/pair#code=ABCD-EFGH", "error": "missing"},
    {"link": "https://emberhut.com/pair#id=&code=ABCD-EFGH", "error": "missing"},
    {"link": "https://emberhut.com/pair#id=" + MACHINE.id, "error": "missing"},
    {"link": "https://emberhut.com/pair#id=" + MACHINE.id + "&code=", "error": "missing"},
    {"link": base + "&id=" + OTHER.id, "error": "repeated"},
    {"link": base + "&code=WXYZ-2345", "error": "repeated"},
    {"link": base + "&route=", "error": "emptyRoute"},
    {"link": base + "".join(f"&route=r{i}" for i in range(9)), "error": "tooManyRoutes"},
    {"link": base + "&x=" + "a" * (2001 - len(base) - 3), "error": "tooLong"},
    {"link": "https://emberhut.com/pair#id=" + MACHINE.id.upper() + "&code=ABCD-EFGH", "error": "id"},
]
assert len(invalid_links[-2]["link"]) == 2001
longest = base + "&x=" + "a" * (2000 - len(base) - 3)
valid_links.append({"link": longest, "id": MACHINE.id, "code": "ABCD-EFGH", "routes": [], "canonical": False})
write("pairing_links.json", {
    "description": "https://emberhut.com/pair#id=<machine id>&code=<code>&route=<url>...: the fragment is application/x-www-form-urlencoded (+ is a space); id and code appear once each, route 0 to 8 times and never empty, unknown keys are ignored, and the link is at most 2,000 characters. A canonical link is exactly what a writer makes from its values (every byte but A-Z a-z 0-9 * - . _ percent-encoded, a space as +, the keys in the order id, code, route...). Errors: notAPairingLink, tooLong, missing, repeated, tooManyRoutes, emptyRoute, id.",
    "valid": valid_links,
    "invalid": invalid_links,
})

# --- Wire messages -----------------------------------------------------------------------------
link_statement = {"publicKey": b64(MACHINE.pk), "name": NAME, "account": ACCOUNT,
                  "relayDomain": DOMAIN, "time": TIME, "signature": b64(link_sig)}
expires = "2026-11-24T22:57:53.410Z"
grant = {"ok": True, "status": 200, "access": "access-token-for-tests",
         "refresh": "refresh-token-for-tests", "device": 3, "expires": expires}
connect = {
    "routeList": {
        "id": MACHINE.id, "publicKey": b64(MACHINE.pk), "name": NAME, "sealed": False,
        "routes": [
            {"kind": "loopback", "url": "http://127.0.0.1:18990", "label": None, "up": True},
            {"kind": "lan", "url": "http://192.0.2.10:18990", "label": None, "up": True},
            {"kind": "vpn", "url": "http://[2001:db8::10]:18990", "label": "WireGuard", "up": False},
            {"kind": "public", "url": "https://emberhut.example", "label": None, "up": True},
            {"kind": "relay", "url": f"https://{RELAY_HOST}", "label": None, "up": False},
            {"kind": "carrier-pigeon", "url": "pigeon://loft", "label": None, "up": True},
        ],
    },
    "pairRequest": {"code": "ABCD-EFGH", "install": "zr-0123456789abcdef", "name": "Emberhut on linux", "model": "linux", "publicKey": b64(DEVICE.pk)},
    "pairAnswer": dict(grant, machine={"id": MACHINE.id, "publicKey": b64(MACHINE.pk)}),
    "pairAnswerLegacy": grant,
    "linkStatementRequest": {"account": ACCOUNT, "relayDomain": DOMAIN},
    "signedLink": link_statement,
    "probeAnswer": answers["lan"],
}
write("connect.json", {
    "description": "Wire JSON of the emberhut server's connect routes and the two pairing fields, as docs/api.md's Connect section gives them, signed by the keys of keys.json. `withUnknownFields` holds each message with a field no version knows added to every object: readers must read it as the plain one. `unreadable` messages must not read, refused by what `refusedBy` names (wrongKey, name, time). `linkChecks` are the directory's checks of signedLink (or of signedLink renamed, its signature unchanged) for a token's account at the directory's own relay domain; `expect` is ok or the directory's error code.",
    "probeQuery": f"nonce={b64(NONCE)}&kind=lan",
    "probeQueryBadKind": f"nonce={b64(NONCE)}&kind=LAN",
    "messages": connect,
    "withUnknownFields": {k: with_unknown_fields(v) for k, v in connect.items()},
    "unreadable": {
        "pairAnswerWrongKey": {"message": dict(grant, machine={"id": OTHER.id, "publicKey": b64(MACHINE.pk)}), "refusedBy": "wrongKey"},
        "routeListWrongKey": {"message": dict(connect["routeList"], id=OTHER.id), "refusedBy": "wrongKey"},
        "routeListLongName": {"message": dict(connect["routeList"], name="x" * 61), "refusedBy": "name"},
        "routeListUntrimmedName": {"message": dict(connect["routeList"], name=" Main"), "refusedBy": "name"},
        "routeListControlInName": {"message": dict(connect["routeList"], name="Main\u202ebox"), "refusedBy": "name"},
        "signedLinkLongName": {"message": dict(link_statement, name="é" * 61), "refusedBy": "name"},
        "signedLinkTimeTooLate": {"message": dict(link_statement, time=MAX_TIME + 1), "refusedBy": "time"},
    },
    "linkChecks": [
        {"link": "signedLink", "account": ACCOUNT, "relayDomain": DOMAIN, "expect": "ok"},
        {"link": "signedLink", "account": "user_2another0account", "relayDomain": DOMAIN, "expect": "not-yours"},
        {"link": "signedLink", "account": ACCOUNT, "relayDomain": "other.example", "expect": "bad-request"},
        {"link": "signedLinkRenamed", "account": ACCOUNT, "relayDomain": DOMAIN, "expect": "bad-signature"},
    ],
    "signedLinkRenamed": dict(link_statement, name="Another name"),
})

auth_msg = statement("emberhut relay auth v1", [DOMAIN, HELLO_NONCE, NAME, TIME])
relay = {
    "hello": {"type": "hello", "version": 1, "nonce": b64(HELLO_NONCE), "time": TIME},
    "auth": {"type": "auth", "publicKey": b64(MACHINE.pk), "name": NAME, "time": TIME, "signature": b64(MACHINE.sign(auth_msg))},
    "welcome": {"type": "welcome", "url": f"https://{RELAY_HOST}", "limits": {"streams": 256, "window": 262144}, "ping": 20},
    "drain": {"type": "drain", "within": 10},
}
write("relay.json", {
    "description": "Control messages of docs/relay.md, version 1, for the relay at relayDomain; the auth answers the hello, and signs the name. Unknown fields and message types are ignored. Close reasons are ASCII key=value pairs separated by ;, at most 123 bytes, with reason=<word> always (a word is visible ASCII without ; or =). Errors: noReason, malformed, word, tooLong.",
    "relayDomain": DOMAIN,
    "messages": relay,
    "withUnknownFields": {k: with_unknown_fields(v) for k, v in relay.items()},
    "unknownType": {"type": "a-later-message", "anything": [1, "two"]},
    "authRenamed": dict(relay["auth"], name="Another name"),
    "closeCodes": [4400, 4401, 4403, 4408, 4409, 4410, 4429, 4502, 1009],
    "closeReasons": {
        "valid": [
            {"text": "reason=deadline", "reason": "deadline"},
            {"text": "reason=clock;time=1795561073000", "reason": "clock", "time": TIME},
            {"text": "reason=limit;retry=30", "reason": "limit", "retry": 30},
            {"text": "reason=revoked;later=1", "reason": "revoked", "written": "reason=revoked"},
            {"text": "reason=" + "x" * 116, "reason": "x" * 116},
        ],
        "invalid": [
            {"text": "time=1795561073000", "error": "noReason"},
            {"text": "reason", "error": "malformed"},
            {"text": "reason=limit;retry=soon", "error": "malformed"},
            {"text": "reason=clock;time=9007199254740992", "error": "malformed"},
            {"text": "reason=", "error": "word"},
            {"text": "reason=a=b", "error": "word"},
            {"text": "reason=a b", "error": "word"},
            {"text": "reason=" + "x" * 117, "error": "tooLong"},
        ],
    },
    "relayErrors": ["unknown-host", "misdirected", "limit", "too-large", "offline", "busy", "full", "timeout"],
})

unlink = {"publicKey": b64(MACHINE.pk), "relayDomain": DOMAIN, "time": TIME, "signature": b64(MACHINE.sign(unlink_msg))}
directory = {
    "machines": {"machines": [
        {"id": MACHINE.id, "publicKey": b64(MACHINE.pk), "name": NAME, "relayUrl": f"wss://{DOMAIN}/v1/sealed/{MACHINE.id}", "connected": True, "lastSeen": TIME, "linkedAt": 1795000000000},
        {"id": OTHER.id, "publicKey": b64(OTHER.pk), "name": "Room 2", "relayUrl": f"wss://{DOMAIN}/v1/sealed/{OTHER.id}", "connected": False, "lastSeen": None, "linkedAt": 1795000000000},
    ]},
    "unlink": unlink,
    "error": {"error": "Sign in to your account again.", "code": "sign-in", "signIn": True},
}
write("directory.json", {
    "description": "Wire JSON of docs/directory.md, version 1. The link body is connect.json's signedLink. `unlinkChecks` are the directory's checks of the unlink posted to /v1/machines/<path>/unlink at its own relay domain; `expect` is ok or the directory's error code.",
    "messages": directory,
    "withUnknownFields": {k: with_unknown_fields(v) for k, v in directory.items()},
    "unreadable": {"machinesWrongKey": {"message": {"machines": [dict(directory["machines"]["machines"][0], id=OTHER.id)]}, "refusedBy": "wrongKey"}},
    "unlinkChecks": [
        {"path": MACHINE.id, "relayDomain": DOMAIN, "expect": "ok"},
        {"path": OTHER.id, "relayDomain": DOMAIN, "expect": "bad-request"},
        {"path": MACHINE.id, "relayDomain": "other.example", "expect": "bad-request"},
    ],
    "errors": [
        {"error": "Sign in to your account again.", "code": "sign-in", "signIn": True},
        {"error": "This machine is linked to another account. Unlink it there first.", "code": "linked-elsewhere"},
        {"error": "This account has all the machines it can hold. Unlink one first.", "code": "account-full"},
        {"error": "Something new.", "code": "a-later-code"},
    ],
})
print("ok", MACHINE.id)
