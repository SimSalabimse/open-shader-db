// SPDX-License-Identifier: MIT

import { canonicalize } from "./canonical.js";
import type { ErrorCode } from "./errors.js";
import { isSafeIdent } from "./keys.js";
import {
  LOCAL_MAX_JSON_BYTES,
  MAX_BYTECODE_HASHES,
  MAX_DESCRIPTION_HASHES,
  MAX_JSON_NESTING,
  MAX_NUMBER_TOKEN_CHARS,
  MAX_PIPELINE_HASHES,
  SCHEMA_VERSION_V1,
  SCHEMA_VERSION_V2,
} from "./limits.js";
import type { OwnershipProof, Sha256Hex, UploadManifest } from "./types.js";

export type ParseResult =
  | {
      readonly ok: true;
      readonly manifest: UploadManifest;
      readonly storedBytes: Uint8Array;
      readonly descriptionHashCount: number;
      readonly bytecodeHashCount: number;
      readonly pipelineHashCount: number;
    }
  | { readonly ok: false; readonly code: ErrorCode };

export interface ParseLimits {
  readonly maxJsonBytes: number;
  readonly maxDescriptionHashes: number;
  readonly maxBytecodeHashes: number;
  readonly maxPipelineHashes: number;
}

export const DEFAULT_PARSE_LIMITS: ParseLimits = {
  maxJsonBytes: LOCAL_MAX_JSON_BYTES,
  maxDescriptionHashes: MAX_DESCRIPTION_HASHES,
  maxBytecodeHashes: MAX_BYTECODE_HASHES,
  maxPipelineHashes: MAX_PIPELINE_HASHES,
};

type Fail = { readonly ok: false; readonly code: ErrorCode };
type Ok<T> = { readonly ok: true; readonly value: T };
type Result<T> = Ok<T> | Fail;

const HEX = "0123456789abcdef";
const HEX_LUT: readonly string[] = Array.from({ length: 256 }, (_, byte) => {
  return `${HEX[byte >> 4] as string}${HEX[byte & 0x0f] as string}`;
});
const EMPTY_HASHES: readonly Sha256Hex[] = Object.freeze([]);
const DECODER = new TextDecoder("utf-8", { fatal: true });

const ROOT_KEYS = new Set([
  "schema_version",
  "build_id",
  "client_version",
  "collected_hour",
  "description_hashes",
  "bytecode_hashes",
  "ownership",
  "pipeline_hashes",
]);
const BUILD_KEYS = new Set(["game_id", "version", "exe_hash"]);
const OWNERSHIP_KEYS = new Set(["kind", "reason", "challenge", "response"]);

class Cursor {
  public compact = true;
  public constructor(
    public readonly bytes: Uint8Array,
    private index = 0,
  ) {}

  public get pos(): number {
    return this.index;
  }

  public get atEnd(): boolean {
    return this.index >= this.bytes.length;
  }

  public peek(): number {
    return this.index < this.bytes.length ? (this.bytes[this.index] as number) : -1;
  }

  public consume(): number {
    const code = this.bytes[this.index] as number;
    this.index += 1;
    return code;
  }

  public skipWs(): void {
    while (this.index < this.bytes.length) {
      const ch = this.bytes[this.index] as number;
      if (ch === 0x20 || ch === 0x0a || ch === 0x0d || ch === 0x09) {
        this.compact = false;
        this.index += 1;
        continue;
      }
      break;
    }
  }

  public startsWith(ascii: string): boolean {
    if (this.index + ascii.length > this.bytes.length) {
      return false;
    }
    for (let i = 0; i < ascii.length; i += 1) {
      if (this.bytes[this.index + i] !== ascii.charCodeAt(i)) {
        return false;
      }
    }
    return true;
  }

  public advance(n: number): void {
    this.index += n;
  }

  public sliceBytes(start: number, end: number): Uint8Array {
    return this.bytes.subarray(start, end);
  }
}

export function parseManifest(bytes: Uint8Array, limits: ParseLimits = DEFAULT_PARSE_LIMITS): ParseResult {
  if (bytes.byteLength > limits.maxJsonBytes) {
    return { ok: false, code: "MANIFEST_TOO_LARGE" };
  }
  if (bytes.byteLength >= 3 && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) {
    return { ok: false, code: "BYTE_ORDER_MARK" };
  }
  if (jsonNestingDepth(bytes) > MAX_JSON_NESTING) {
    return { ok: false, code: "NESTING_TOO_DEEP" };
  }
  const cur = new Cursor(bytes);
  cur.skipWs();
  if (cur.atEnd) {
    return { ok: false, code: "INVALID_JSON" };
  }
  if (cur.peek() !== 0x7b) {
    return { ok: false, code: "INVALID_JSON" };
  }
  const parsed = parseRootObject(cur, limits);
  if (!parsed.ok) {
    return parsed;
  }
  cur.skipWs();
  if (!cur.atEnd) {
    return { ok: false, code: "TRAILING_GARBAGE" };
  }
  const descriptionHashes = cur.compact ? EMPTY_HASHES : hexList(parsed.value.desc.raw, parsed.value.desc.count);
  const bytecodeHashes = cur.compact ? EMPTY_HASHES : hexList(parsed.value.code.raw, parsed.value.code.count);
  const schemaVersion = parsed.value.schemaVersion;
  const pipelineCount = parsed.value.pipeline?.count ?? 0;
  const manifest: UploadManifest = {
    schema_version: schemaVersion,
    build_id: parsed.value.buildId,
    client_version: parsed.value.clientVersion,
    collected_hour: parsed.value.collectedHour,
    description_hashes: descriptionHashes,
    bytecode_hashes: bytecodeHashes,
    ownership: parsed.value.ownership,
  };
  if (schemaVersion === SCHEMA_VERSION_V2) {
    const pipelineHashes = cur.compact
      ? EMPTY_HASHES
      : hexList(parsed.value.pipeline?.raw ?? new Uint8Array(), pipelineCount);
    return {
      ok: true,
      manifest: { ...manifest, pipeline_hashes: pipelineHashes },
      storedBytes: cur.compact ? bytes : canonicalize({ ...manifest, pipeline_hashes: pipelineHashes }),
      descriptionHashCount: parsed.value.desc.count,
      bytecodeHashCount: parsed.value.code.count,
      pipelineHashCount: pipelineCount,
    };
  }
  const storedBytes = cur.compact ? bytes : canonicalize(manifest);
  return {
    ok: true,
    manifest,
    storedBytes,
    descriptionHashCount: parsed.value.desc.count,
    bytecodeHashCount: parsed.value.code.count,
    pipelineHashCount: 0,
  };
}

function jsonNestingDepth(bytes: Uint8Array): number {
  let depth = 0;
  let max = 0;
  let inStr = false;
  let escape = false;
  for (let i = 0; i < bytes.length; i += 1) {
    const byte = bytes[i] as number;
    if (inStr) {
      if (escape) {
        escape = false;
        continue;
      }
      if (byte === 0x5c) {
        escape = true;
        continue;
      }
      if (byte === 0x22) {
        inStr = false;
      }
      continue;
    }
    if (byte === 0x22) {
      inStr = true;
    } else if (byte === 0x7b || byte === 0x5b) {
      depth += 1;
      if (depth > max) {
        max = depth;
      }
    } else if (byte === 0x7d || byte === 0x5d) {
      depth = depth > 0 ? depth - 1 : 0;
    }
  }
  return max;
}

interface HashList {
  readonly count: number;
  readonly raw: Uint8Array;
}

interface RootFields {
  readonly schemaVersion: 1 | 2;
  readonly buildId: UploadManifest["build_id"];
  readonly clientVersion: string;
  readonly collectedHour: string;
  readonly desc: HashList;
  readonly code: HashList;
  readonly ownership: OwnershipProof;
  readonly pipeline: HashList | undefined;
}

function parseRootObject(cur: Cursor, limits: ParseLimits): Result<RootFields> {
  cur.consume();
  const seen = new Set<string>();
  let lastKey = "";
  let schemaVersion: 1 | 2 | undefined;
  let buildId: UploadManifest["build_id"] | undefined;
  let clientVersion: string | undefined;
  let collectedHour: string | undefined;
  let desc: HashList | undefined;
  let code: HashList | undefined;
  let ownership: OwnershipProof | undefined;
  let pipeline: HashList | undefined;
  let pipelineSpan: { readonly start: number; readonly end: number } | undefined;
  cur.skipWs();
  if (cur.peek() === 0x7d) {
    cur.consume();
    return { ok: false, code: "MISSING_FIELD" };
  }
  for (;;) {
    cur.skipWs();
    const key = parseObjectKey(cur, ROOT_KEYS);
    if (!key.ok) {
      return key;
    }
    if (seen.has(key.value)) {
      return { ok: false, code: "DUPLICATE_KEY" };
    }
    if (lastKey !== "" && key.value < lastKey) {
      cur.compact = false;
    }
    lastKey = key.value;
    seen.add(key.value);
    cur.skipWs();
    if (cur.peek() !== 0x3a) {
      return { ok: false, code: "INVALID_JSON" };
    }
    cur.consume();
    cur.skipWs();
    switch (key.value) {
      case "schema_version": {
        const value = parseSchemaVersion(cur);
        if (!value.ok) {
          return value;
        }
        schemaVersion = value.value;
        if (schemaVersion === SCHEMA_VERSION_V1 && (pipeline !== undefined || pipelineSpan !== undefined)) {
          return { ok: false, code: "UNKNOWN_FIELD" };
        }
        if (schemaVersion === SCHEMA_VERSION_V2 && pipelineSpan !== undefined) {
          const slice = cur.bytes.subarray(pipelineSpan.start, pipelineSpan.end);
          const parsedList = parseHashList(new Cursor(slice), limits.maxPipelineHashes);
          if (!parsedList.ok) {
            return parsedList;
          }
          pipeline = parsedList.value;
          pipelineSpan = undefined;
        }
        break;
      }
      case "build_id": {
        const value = parseBuildId(cur);
        if (!value.ok) {
          return value;
        }
        buildId = value.value;
        break;
      }
      case "client_version": {
        const value = parseClientVersion(cur);
        if (!value.ok) {
          return value;
        }
        clientVersion = value.value;
        break;
      }
      case "collected_hour": {
        const value = parseUtcHour(cur);
        if (!value.ok) {
          return value;
        }
        collectedHour = value.value;
        break;
      }
      case "description_hashes": {
        const value = parseHashList(cur, limits.maxDescriptionHashes);
        if (!value.ok) {
          return value;
        }
        desc = value.value;
        break;
      }
      case "bytecode_hashes": {
        const value = parseHashList(cur, limits.maxBytecodeHashes);
        if (!value.ok) {
          return value;
        }
        code = value.value;
        break;
      }
      case "ownership": {
        const value = parseOwnership(cur);
        if (!value.ok) {
          return value;
        }
        ownership = value.value;
        break;
      }
      case "pipeline_hashes": {
        if (schemaVersion === SCHEMA_VERSION_V1) {
          return { ok: false, code: "UNKNOWN_FIELD" };
        }
        if (schemaVersion === SCHEMA_VERSION_V2) {
          const value = parseHashList(cur, limits.maxPipelineHashes);
          if (!value.ok) {
            return value;
          }
          pipeline = value.value;
          break;
        }
        const start = cur.pos;
        const skipped = skipJsonValue(cur);
        if (!skipped.ok) {
          return skipped;
        }
        pipelineSpan = { start, end: cur.pos };
        break;
      }
      default:
        return { ok: false, code: "UNKNOWN_FIELD" };
    }
    cur.skipWs();
    const next = cur.peek();
    if (next === 0x2c) {
      cur.consume();
      continue;
    }
    if (next === 0x7d) {
      cur.consume();
      if (
        schemaVersion === undefined ||
        buildId === undefined ||
        clientVersion === undefined ||
        collectedHour === undefined ||
        desc === undefined ||
        code === undefined ||
        ownership === undefined
      ) {
        return { ok: false, code: "MISSING_FIELD" };
      }
      if (schemaVersion === SCHEMA_VERSION_V2 && pipeline === undefined) {
        return { ok: false, code: "MISSING_FIELD" };
      }
      if (schemaVersion === SCHEMA_VERSION_V1 && (pipeline !== undefined || pipelineSpan !== undefined)) {
        return { ok: false, code: "UNKNOWN_FIELD" };
      }
      return {
        ok: true,
        value: { schemaVersion, buildId, clientVersion, collectedHour, desc, code, ownership, pipeline },
      };
    }
    return { ok: false, code: "INVALID_JSON" };
  }
}

function parseObjectKey(cur: Cursor, allowed: ReadonlySet<string>): Result<string> {
  const parsed = parseString(cur, 32, "UNKNOWN_FIELD");
  if (!parsed.ok) {
    return parsed;
  }
  if (!allowed.has(parsed.value)) {
    return { ok: false, code: "UNKNOWN_FIELD" };
  }
  return parsed;
}

function parseSchemaVersion(cur: Cursor): Result<1 | 2> {
  const num = parseNumber(cur);
  if (!num.ok) {
    return num;
  }
  if (num.value.kind === "float") {
    return { ok: false, code: "INVALID_INTEGER" };
  }
  if (num.value.value === SCHEMA_VERSION_V1 || num.value.value === SCHEMA_VERSION_V2) {
    return { ok: true, value: num.value.value };
  }
  return { ok: false, code: "UNSUPPORTED_SCHEMA_VERSION" };
}

function skipJsonValue(cur: Cursor): Result<void> {
  cur.skipWs();
  const ch = cur.peek();
  if (ch === 0x22) {
    return skipString(cur);
  }
  if (ch === 0x7b) {
    return skipContainer(cur, 0x7b, 0x7d);
  }
  if (ch === 0x5b) {
    return skipContainer(cur, 0x5b, 0x5d);
  }
  if (ch === 0x74) {
    return skipLiteral(cur, "true");
  }
  if (ch === 0x66) {
    return skipLiteral(cur, "false");
  }
  if (ch === 0x6e) {
    return skipLiteral(cur, "null");
  }
  if (ch === 0x2d || (ch >= 0x30 && ch <= 0x39)) {
    return skipNumber(cur);
  }
  return { ok: false, code: "INVALID_JSON" };
}

function skipContainer(cur: Cursor, open: number, close: number): Result<void> {
  if (cur.peek() !== open) {
    return { ok: false, code: "INVALID_JSON" };
  }
  cur.consume();
  cur.skipWs();
  if (cur.peek() === close) {
    cur.consume();
    return { ok: true, value: undefined };
  }
  for (;;) {
    if (open === 0x7b) {
      const key = skipString(cur);
      if (!key.ok) {
        return key;
      }
      cur.skipWs();
      if (cur.peek() !== 0x3a) {
        return { ok: false, code: "INVALID_JSON" };
      }
      cur.consume();
    }
    const value = skipJsonValue(cur);
    if (!value.ok) {
      return value;
    }
    cur.skipWs();
    const next = cur.peek();
    if (next === 0x2c) {
      cur.consume();
      cur.skipWs();
      continue;
    }
    if (next === close) {
      cur.consume();
      return { ok: true, value: undefined };
    }
    return { ok: false, code: "INVALID_JSON" };
  }
}

function skipString(cur: Cursor): Result<void> {
  if (cur.peek() !== 0x22) {
    return { ok: false, code: "INVALID_JSON" };
  }
  cur.consume();
  while (!cur.atEnd) {
    const code = cur.peek();
    if (code === 0x22) {
      cur.consume();
      return { ok: true, value: undefined };
    }
    if (code === 0x5c) {
      cur.compact = false;
      cur.consume();
      if (!consumeEscape(cur)) {
        return { ok: false, code: "INVALID_JSON" };
      }
      continue;
    }
    if (code < 0x20) {
      return { ok: false, code: "INVALID_JSON" };
    }
    const width = utf8Width(code);
    if (width < 1 || !consumeUtf8(cur, width)) {
      return { ok: false, code: "INVALID_JSON" };
    }
  }
  return { ok: false, code: "INVALID_JSON" };
}

function skipLiteral(cur: Cursor, ascii: string): Result<void> {
  if (!cur.startsWith(ascii)) {
    return { ok: false, code: "INVALID_JSON" };
  }
  cur.advance(ascii.length);
  return { ok: true, value: undefined };
}

function skipNumber(cur: Cursor): Result<void> {
  if (cur.peek() === 0x2d) {
    cur.consume();
  }
  if (cur.atEnd || cur.peek() < 0x30 || cur.peek() > 0x39) {
    return { ok: false, code: "INVALID_JSON" };
  }
  if (cur.peek() === 0x30) {
    cur.consume();
    if (!cur.atEnd && cur.peek() >= 0x30 && cur.peek() <= 0x39) {
      return { ok: false, code: "INVALID_JSON" };
    }
  } else {
    while (!cur.atEnd && cur.peek() >= 0x30 && cur.peek() <= 0x39) {
      cur.consume();
    }
  }
  if (!cur.atEnd && cur.peek() === 0x2e) {
    cur.consume();
    if (cur.atEnd || cur.peek() < 0x30 || cur.peek() > 0x39) {
      return { ok: false, code: "INVALID_JSON" };
    }
    while (!cur.atEnd && cur.peek() >= 0x30 && cur.peek() <= 0x39) {
      cur.consume();
    }
  }
  if (!cur.atEnd && (cur.peek() === 0x65 || cur.peek() === 0x45)) {
    cur.consume();
    if (!cur.atEnd && (cur.peek() === 0x2b || cur.peek() === 0x2d)) {
      cur.consume();
    }
    if (cur.atEnd || cur.peek() < 0x30 || cur.peek() > 0x39) {
      return { ok: false, code: "INVALID_JSON" };
    }
    while (!cur.atEnd && cur.peek() >= 0x30 && cur.peek() <= 0x39) {
      cur.consume();
    }
  }
  return { ok: true, value: undefined };
}

function parseBuildId(cur: Cursor): Result<UploadManifest["build_id"]> {
  if (cur.peek() !== 0x7b) {
    return { ok: false, code: "INVALID_JSON" };
  }
  cur.consume();
  const seen = new Set<string>();
  let lastKey = "";
  let gameId: string | undefined;
  let version: string | undefined;
  let exeHash: Sha256Hex | undefined;
  cur.skipWs();
  if (cur.peek() === 0x7d) {
    cur.consume();
    return { ok: false, code: "MISSING_FIELD" };
  }
  for (;;) {
    cur.skipWs();
    const key = parseObjectKey(cur, BUILD_KEYS);
    if (!key.ok) {
      return key;
    }
    if (seen.has(key.value)) {
      return { ok: false, code: "DUPLICATE_KEY" };
    }
    if (lastKey !== "" && key.value < lastKey) {
      cur.compact = false;
    }
    lastKey = key.value;
    seen.add(key.value);
    cur.skipWs();
    if (cur.peek() !== 0x3a) {
      return { ok: false, code: "INVALID_JSON" };
    }
    cur.consume();
    cur.skipWs();
    if (key.value === "exe_hash") {
      const hex = parseHexString(cur);
      if (!hex.ok) {
        return hex;
      }
      exeHash = hex.value;
    } else {
      const text = parseString(cur, 128, "INVALID_BUILD_ID");
      if (!text.ok) {
        return text;
      }
      if (!isSafeIdent(text.value)) {
        return { ok: false, code: "INVALID_BUILD_ID" };
      }
      if (key.value === "game_id") {
        gameId = text.value;
      } else {
        version = text.value;
      }
    }
    cur.skipWs();
    const next = cur.peek();
    if (next === 0x2c) {
      cur.consume();
      continue;
    }
    if (next === 0x7d) {
      cur.consume();
      if (gameId === undefined || version === undefined || exeHash === undefined) {
        return { ok: false, code: "MISSING_FIELD" };
      }
      return { ok: true, value: { game_id: gameId, version, exe_hash: exeHash } };
    }
    return { ok: false, code: "INVALID_JSON" };
  }
}

function parseClientVersion(cur: Cursor): Result<string> {
  if (cur.peek() !== 0x22) {
    return { ok: false, code: "INVALID_CLIENT_VERSION" };
  }
  const text = parseString(cur, 32, "INVALID_CLIENT_VERSION");
  if (!text.ok) {
    return text;
  }
  if (text.value.length === 0 || text.value.length > 32) {
    return { ok: false, code: "INVALID_CLIENT_VERSION" };
  }
  const parts = text.value.split(".");
  if (parts.length !== 3) {
    return { ok: false, code: "INVALID_CLIENT_VERSION" };
  }
  for (const part of parts) {
    if (part.length === 0) {
      return { ok: false, code: "INVALID_CLIENT_VERSION" };
    }
    for (let i = 0; i < part.length; i += 1) {
      const ch = part.charCodeAt(i);
      if (ch < 0x30 || ch > 0x39) {
        return { ok: false, code: "INVALID_CLIENT_VERSION" };
      }
    }
  }
  return { ok: true, value: text.value };
}

function parseUtcHour(cur: Cursor): Result<string> {
  if (cur.peek() !== 0x22) {
    return { ok: false, code: "INVALID_TIMESTAMP" };
  }
  const text = parseString(cur, 20, "INVALID_TIMESTAMP");
  if (!text.ok) {
    return text;
  }
  if (text.value.length !== 20) {
    return { ok: false, code: "INVALID_TIMESTAMP" };
  }
  const match = /^([0-9]{4})-([0-9]{2})-([0-9]{2})T([0-9]{2}):00:00Z$/.exec(text.value);
  if (match === null) {
    return { ok: false, code: "INVALID_TIMESTAMP" };
  }
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const hour = Number(match[4]);
  if (year < 1970 || year > 9999 || hour > 23) {
    return { ok: false, code: "INVALID_TIMESTAMP" };
  }
  if (month < 1 || month > 12 || day < 1 || day > daysInMonth(year, month)) {
    return { ok: false, code: "INVALID_TIMESTAMP" };
  }
  return { ok: true, value: text.value };
}

function daysInMonth(year: number, month: number): number {
  switch (month) {
    case 1:
    case 3:
    case 5:
    case 7:
    case 8:
    case 10:
    case 12:
      return 31;
    case 4:
    case 6:
    case 9:
    case 11:
      return 30;
    case 2:
      return isLeapYear(year) ? 29 : 28;
    default:
      return 0;
  }
}

function isLeapYear(year: number): boolean {
  return year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
}

function parseHashList(cur: Cursor, max: number): Result<HashList> {
  if (cur.peek() !== 0x5b) {
    return { ok: false, code: "INVALID_JSON" };
  }
  cur.consume();
  cur.skipWs();
  if (cur.peek() === 0x5d) {
    cur.consume();
    return { ok: true, value: { count: 0, raw: new Uint8Array() } };
  }
  let raw = new Uint8Array(32 * Math.min(max, 8));
  const digest = new Uint8Array(32);
  const prev = new Uint8Array(32);
  let havePrev = false;
  let count = 0;
  for (;;) {
    if (count >= max) {
      return { ok: false, code: "TOO_MANY_HASHES" };
    }
    cur.skipWs();
    const item = parseHexDigest(cur, digest);
    if (!item.ok) {
      return item;
    }
    if (havePrev && compareBytes(prev, digest) >= 0) {
      return { ok: false, code: "UNSORTED_OR_DUPLICATE_HASHES" };
    }
    if (count * 32 === raw.length) {
      const grown = new Uint8Array(Math.min(max, Math.max(8, count * 2)) * 32);
      grown.set(raw);
      raw = grown;
    }
    raw.set(digest, count * 32);
    prev.set(digest);
    havePrev = true;
    count += 1;
    cur.skipWs();
    const next = cur.peek();
    if (next === 0x2c) {
      cur.consume();
      continue;
    }
    if (next === 0x5d) {
      cur.consume();
      return { ok: true, value: { count, raw: raw.subarray(0, count * 32) } };
    }
    return { ok: false, code: "INVALID_JSON" };
  }
}

function parseOwnership(cur: Cursor): Result<OwnershipProof> {
  if (cur.peek() !== 0x7b) {
    return { ok: false, code: "INVALID_JSON" };
  }
  cur.consume();
  const seen = new Set<string>();
  let lastKey = "";
  let kind: string | undefined;
  let reason: string | undefined;
  let challenge: Sha256Hex | undefined;
  let response: Sha256Hex | undefined;
  cur.skipWs();
  if (cur.peek() === 0x7d) {
    cur.consume();
    return { ok: false, code: "MISSING_FIELD" };
  }
  for (;;) {
    cur.skipWs();
    const key = parseObjectKey(cur, OWNERSHIP_KEYS);
    if (!key.ok) {
      return key;
    }
    if (seen.has(key.value)) {
      return { ok: false, code: "DUPLICATE_KEY" };
    }
    if (lastKey !== "" && key.value < lastKey) {
      cur.compact = false;
    }
    lastKey = key.value;
    seen.add(key.value);
    cur.skipWs();
    if (cur.peek() !== 0x3a) {
      return { ok: false, code: "INVALID_JSON" };
    }
    cur.consume();
    cur.skipWs();
    if (key.value === "challenge" || key.value === "response") {
      const hex = parseHexString(cur);
      if (!hex.ok) {
        return hex;
      }
      if (key.value === "challenge") {
        challenge = hex.value;
      } else {
        response = hex.value;
      }
    } else {
      const text = parseString(cur, 32, "UNKNOWN_OWNERSHIP_KIND");
      if (!text.ok) {
        return text;
      }
      if (key.value === "kind") {
        kind = text.value;
      } else {
        reason = text.value;
      }
    }
    cur.skipWs();
    const next = cur.peek();
    if (next === 0x2c) {
      cur.consume();
      continue;
    }
    if (next === 0x7d) {
      cur.consume();
      return finishOwnership(kind, reason, challenge, response);
    }
    return { ok: false, code: "INVALID_JSON" };
  }
}

function finishOwnership(
  kind: string | undefined,
  reason: string | undefined,
  challenge: Sha256Hex | undefined,
  response: Sha256Hex | undefined,
): Result<OwnershipProof> {
  if (kind === undefined) {
    return { ok: false, code: "MISSING_FIELD" };
  }
  if (kind === "unchallenged") {
    if (challenge !== undefined || response !== undefined) {
      return { ok: false, code: "UNKNOWN_FIELD" };
    }
    if (reason === undefined) {
      return { ok: false, code: "MISSING_FIELD" };
    }
    if (reason !== "no_server_challenge") {
      return { ok: false, code: "UNKNOWN_OWNERSHIP_REASON" };
    }
    return { ok: true, value: { kind: "unchallenged", reason: "no_server_challenge" } };
  }
  if (kind === "exe_binding_v1") {
    if (reason !== undefined) {
      return { ok: false, code: "UNKNOWN_FIELD" };
    }
    if (challenge === undefined || response === undefined) {
      return { ok: false, code: "MISSING_FIELD" };
    }
    return { ok: true, value: { kind: "exe_binding_v1", challenge, response } };
  }
  return { ok: false, code: "UNKNOWN_OWNERSHIP_KIND" };
}

function parseString(cur: Cursor, maxLen: number, overCode: ErrorCode): Result<string> {
  if (cur.peek() !== 0x22) {
    return { ok: false, code: "INVALID_JSON" };
  }
  cur.consume();
  const start = cur.pos;
  let escaped = false;
  let chars = 0;
  while (!cur.atEnd) {
    const code = cur.peek();
    if (code === 0x22) {
      const raw = cur.sliceBytes(start, cur.pos);
      cur.consume();
      if (chars > maxLen) {
        return { ok: false, code: overCode };
      }
      if (!escaped) {
        try {
          return { ok: true, value: DECODER.decode(raw) };
        } catch {
          return { ok: false, code: "INVALID_JSON" };
        }
      }
      const rebuilt = unescapeBytes(raw);
      if (rebuilt === null) {
        return { ok: false, code: "INVALID_JSON" };
      }
      if (rebuilt.length > maxLen) {
        return { ok: false, code: overCode };
      }
      cur.compact = false;
      return { ok: true, value: rebuilt };
    }
    if (code === 0x5c) {
      escaped = true;
      cur.compact = false;
      cur.consume();
      if (!consumeEscape(cur)) {
        return { ok: false, code: "INVALID_JSON" };
      }
      chars += 1;
      if (chars > maxLen) {
        return { ok: false, code: overCode };
      }
      continue;
    }
    if (code < 0x20) {
      return { ok: false, code: "INVALID_JSON" };
    }
    const width = utf8Width(code);
    if (width < 1 || !consumeUtf8(cur, width)) {
      return { ok: false, code: "INVALID_JSON" };
    }
    chars += 1;
    if (chars > maxLen) {
      return { ok: false, code: overCode };
    }
  }
  return { ok: false, code: "INVALID_JSON" };
}

function parseHexString(cur: Cursor): Result<Sha256Hex> {
  const digest = new Uint8Array(32);
  const parsed = parseHexDigest(cur, digest);
  if (!parsed.ok) {
    return parsed;
  }
  return { ok: true, value: hexEncode32(digest) };
}

function parseHexDigest(cur: Cursor, out: Uint8Array): Result<void> {
  if (cur.peek() !== 0x22) {
    return { ok: false, code: "INVALID_JSON" };
  }
  cur.consume();
  if (cur.pos + 64 <= cur.bytes.length && cur.bytes[cur.pos + 64] === 0x22) {
    const slice = cur.sliceBytes(cur.pos, cur.pos + 64);
    if (decodeHex32(slice, out)) {
      cur.advance(65);
      return { ok: true, value: undefined };
    }
  }
  const text = parseStringFromCurrent(cur, 256, "WRONG_HEX_LENGTH");
  if (!text.ok) {
    return text;
  }
  const parsed = parseHex(text.value);
  if (!parsed.ok) {
    return parsed;
  }
  decodeHex32(asciiBytes(parsed.value), out);
  return { ok: true, value: undefined };
}

function parseStringFromCurrent(cur: Cursor, maxLen: number, overCode: ErrorCode): Result<string> {
  const start = cur.pos;
  let escaped = false;
  let chars = 0;
  while (!cur.atEnd) {
    const code = cur.peek();
    if (code === 0x22) {
      const raw = cur.sliceBytes(start, cur.pos);
      cur.consume();
      if (chars > maxLen) {
        return { ok: false, code: overCode };
      }
      if (!escaped) {
        try {
          return { ok: true, value: DECODER.decode(raw) };
        } catch {
          return { ok: false, code: "INVALID_JSON" };
        }
      }
      const rebuilt = unescapeBytes(raw);
      if (rebuilt === null) {
        return { ok: false, code: "INVALID_JSON" };
      }
      cur.compact = false;
      return { ok: true, value: rebuilt };
    }
    if (code === 0x5c) {
      escaped = true;
      cur.compact = false;
      cur.consume();
      if (!consumeEscape(cur)) {
        return { ok: false, code: "INVALID_JSON" };
      }
      chars += 1;
      if (chars > maxLen) {
        return { ok: false, code: overCode };
      }
      continue;
    }
    if (code < 0x20) {
      return { ok: false, code: "INVALID_JSON" };
    }
    const width = utf8Width(code);
    if (width < 1 || !consumeUtf8(cur, width)) {
      return { ok: false, code: "INVALID_JSON" };
    }
    chars += 1;
    if (chars > maxLen) {
      return { ok: false, code: overCode };
    }
  }
  return { ok: false, code: "INVALID_JSON" };
}

function consumeEscape(cur: Cursor): boolean {
  if (cur.atEnd) {
    return false;
  }
  const code = cur.consume();
  switch (code) {
    case 0x22:
    case 0x5c:
    case 0x2f:
    case 0x62:
    case 0x66:
    case 0x6e:
    case 0x72:
    case 0x74:
      return true;
    case 0x75: {
      if (cur.pos + 4 > cur.bytes.length) {
        return false;
      }
      for (let i = 0; i < 4; i += 1) {
        if (hexDigit(cur.consume()) < 0) {
          return false;
        }
      }
      return true;
    }
    default:
      return false;
  }
}

function unescapeBytes(raw: Uint8Array): string | null {
  const parts: string[] = [];
  let run = 0;
  let i = 0;
  while (i < raw.length) {
    const code = raw[i] as number;
    if (code !== 0x5c) {
      i += 1;
      continue;
    }
    if (i > run) {
      try {
        parts.push(DECODER.decode(raw.subarray(run, i)));
      } catch {
        return null;
      }
    }
    i += 1;
    if (i >= raw.length) {
      return null;
    }
    const esc = raw[i] as number;
    switch (esc) {
      case 0x22:
        parts.push('"');
        break;
      case 0x5c:
        parts.push("\\");
        break;
      case 0x2f:
        parts.push("/");
        break;
      case 0x62:
        parts.push("\b");
        break;
      case 0x66:
        parts.push("\f");
        break;
      case 0x6e:
        parts.push("\n");
        break;
      case 0x72:
        parts.push("\r");
        break;
      case 0x74:
        parts.push("\t");
        break;
      case 0x75: {
        if (i + 4 >= raw.length) {
          return null;
        }
        let value = 0;
        for (let n = 0; n < 4; n += 1) {
          i += 1;
          const nibble = hexDigit(raw[i] as number);
          if (nibble < 0) {
            return null;
          }
          value = (value << 4) | nibble;
        }
        if (value >= 0xd800 && value <= 0xdfff) {
          return null;
        }
        parts.push(String.fromCharCode(value));
        break;
      }
      default:
        return null;
    }
    i += 1;
    run = i;
  }
  if (run < raw.length) {
    try {
      parts.push(DECODER.decode(raw.subarray(run)));
    } catch {
      return null;
    }
  }
  return parts.join("");
}

function utf8Width(byte: number): number {
  if (byte <= 0x7f) {
    return 1;
  }
  if (byte >= 0xc2 && byte <= 0xdf) {
    return 2;
  }
  if (byte >= 0xe0 && byte <= 0xef) {
    return 3;
  }
  if (byte >= 0xf0 && byte <= 0xf4) {
    return 4;
  }
  return -1;
}

function consumeUtf8(cur: Cursor, width: number): boolean {
  if (cur.pos + width > cur.bytes.length) {
    return false;
  }
  const first = cur.consume();
  for (let i = 1; i < width; i += 1) {
    const next = cur.consume();
    if ((next & 0xc0) !== 0x80) {
      return false;
    }
    void first;
  }
  return true;
}

function parseNumber(cur: Cursor): Result<{ kind: "int"; value: number } | { kind: "float" }> {
  const start = cur.pos;
  if (cur.peek() === 0x2d) {
    cur.consume();
  }
  if (cur.atEnd || cur.peek() < 0x30 || cur.peek() > 0x39) {
    return { ok: false, code: "INVALID_JSON" };
  }
  if (cur.peek() === 0x30) {
    cur.consume();
    if (!cur.atEnd && cur.peek() >= 0x30 && cur.peek() <= 0x39) {
      return { ok: false, code: "INVALID_JSON" };
    }
  } else {
    while (!cur.atEnd && cur.peek() >= 0x30 && cur.peek() <= 0x39) {
      cur.consume();
      if (cur.pos - start > MAX_NUMBER_TOKEN_CHARS) {
        return { ok: false, code: "INVALID_INTEGER" };
      }
    }
  }
  let isFloat = false;
  if (!cur.atEnd && cur.peek() === 0x2e) {
    isFloat = true;
    cur.consume();
    if (cur.atEnd || cur.peek() < 0x30 || cur.peek() > 0x39) {
      return { ok: false, code: "INVALID_JSON" };
    }
    while (!cur.atEnd && cur.peek() >= 0x30 && cur.peek() <= 0x39) {
      cur.consume();
      if (cur.pos - start > MAX_NUMBER_TOKEN_CHARS) {
        return { ok: false, code: "INVALID_INTEGER" };
      }
    }
  }
  if (!cur.atEnd && (cur.peek() === 0x65 || cur.peek() === 0x45)) {
    isFloat = true;
    cur.consume();
    if (!cur.atEnd && (cur.peek() === 0x2b || cur.peek() === 0x2d)) {
      cur.consume();
    }
    if (cur.atEnd || cur.peek() < 0x30 || cur.peek() > 0x39) {
      return { ok: false, code: "INVALID_JSON" };
    }
    while (!cur.atEnd && cur.peek() >= 0x30 && cur.peek() <= 0x39) {
      cur.consume();
      if (cur.pos - start > MAX_NUMBER_TOKEN_CHARS) {
        return { ok: false, code: "INVALID_INTEGER" };
      }
    }
  }
  if (cur.pos - start > MAX_NUMBER_TOKEN_CHARS) {
    return { ok: false, code: "INVALID_INTEGER" };
  }
  const numberToken = asciiString(cur.sliceBytes(start, cur.pos));
  if (isFloat || numberToken.startsWith("-")) {
    return { ok: true, value: { kind: "float" } };
  }
  const value = Number(numberToken);
  if (!Number.isSafeInteger(value)) {
    return { ok: true, value: { kind: "float" } };
  }
  return { ok: true, value: { kind: "int", value } };
}

function parseHex(text: string): Result<Sha256Hex> {
  if (text.length !== 64) {
    return { ok: false, code: "WRONG_HEX_LENGTH" };
  }
  for (let i = 0; i < text.length; i += 1) {
    const ch = text.charCodeAt(i);
    if (!((ch >= 0x30 && ch <= 0x39) || (ch >= 0x61 && ch <= 0x66))) {
      return { ok: false, code: "MALFORMED_HEX" };
    }
  }
  return { ok: true, value: text };
}

function asciiString(bytes: Uint8Array): string {
  const parts = new Array<string>(bytes.length);
  for (let i = 0; i < bytes.length; i += 1) {
    parts[i] = String.fromCharCode(bytes[i] as number);
  }
  return parts.join("");
}

function hexEncode32(bytes: Uint8Array): Sha256Hex {
  const parts = new Array<string>(32);
  for (let i = 0; i < 32; i += 1) {
    parts[i] = HEX_LUT[bytes[i] as number] as string;
  }
  return parts.join("");
}

function hexList(raw: Uint8Array, count: number): Sha256Hex[] {
  const out = new Array<Sha256Hex>(count);
  const digest = new Uint8Array(32);
  for (let i = 0; i < count; i += 1) {
    digest.set(raw.subarray(i * 32, i * 32 + 32));
    out[i] = hexEncode32(digest);
  }
  return out;
}

function decodeHex32(hex: Uint8Array, out: Uint8Array): boolean {
  if (hex.length !== 64) {
    return false;
  }
  for (let i = 0; i < 32; i += 1) {
    const hi = hexDigit(hex[i * 2] as number);
    const lo = hexDigit(hex[i * 2 + 1] as number);
    if (hi < 0 || lo < 0 || hi > 15 || lo > 15) {
      return false;
    }
    if (hi >= 10 && (hex[i * 2] as number) < 0x61) {
      return false;
    }
    if (lo >= 10 && (hex[i * 2 + 1] as number) < 0x61) {
      return false;
    }
    out[i] = (hi << 4) | lo;
  }
  return true;
}

function asciiBytes(text: string): Uint8Array {
  const out = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i += 1) {
    out[i] = text.charCodeAt(i);
  }
  return out;
}

function compareBytes(left: Uint8Array, right: Uint8Array): number {
  const n = Math.min(left.length, right.length);
  for (let i = 0; i < n; i += 1) {
    const d = (left[i] as number) - (right[i] as number);
    if (d !== 0) {
      return d;
    }
  }
  return left.length - right.length;
}

function hexDigit(code: number): number {
  if (code >= 0x30 && code <= 0x39) {
    return code - 0x30;
  }
  if (code >= 0x61 && code <= 0x66) {
    return code - 0x61 + 10;
  }
  if (code >= 0x41 && code <= 0x46) {
    return code - 0x41 + 10;
  }
  return -1;
}
