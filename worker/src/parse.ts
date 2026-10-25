// SPDX-License-Identifier: MIT

import type { ErrorCode } from "./errors.js";
import { isSafeIdent } from "./keys.js";
import {
  MAX_BYTECODE_HASHES,
  MAX_DESCRIPTION_HASHES,
  MAX_JSON_NESTING,
  SCHEMA_VERSION,
} from "./limits.js";
import type { OwnershipProof, Sha256Hex, UploadManifest } from "./types.js";

export type ParseResult =
  | { readonly ok: true; readonly manifest: UploadManifest }
  | { readonly ok: false; readonly code: ErrorCode };

type Fail = { readonly ok: false; readonly code: ErrorCode };
type Ok<T> = { readonly ok: true; readonly value: T };
type Result<T> = Ok<T> | Fail;

type Json =
  | { readonly kind: "object"; readonly entries: ReadonlyMap<string, Json> }
  | { readonly kind: "array"; readonly items: readonly Json[] }
  | { readonly kind: "string"; readonly value: string }
  | { readonly kind: "int"; readonly value: number }
  | { readonly kind: "float" }
  | { readonly kind: "bool"; readonly value: boolean }
  | { readonly kind: "null" };

class Cursor {
  public constructor(
    private readonly text: string,
    private index = 0,
  ) {}

  public get pos(): number {
    return this.index;
  }

  public get atEnd(): boolean {
    return this.index >= this.text.length;
  }

  public skipWs(): void {
    while (this.index < this.text.length) {
      const ch = this.text.charCodeAt(this.index);
      if (ch === 0x20 || ch === 0x0a || ch === 0x0d || ch === 0x09) {
        this.index += 1;
        continue;
      }
      break;
    }
  }

  public peek(): number {
    return this.text.charCodeAt(this.index);
  }

  public consume(): number {
    const code = this.text.charCodeAt(this.index);
    this.index += 1;
    return code;
  }

  public startsWith(text: string): boolean {
    return this.text.startsWith(text, this.index);
  }

  public advance(n: number): void {
    this.index += n;
  }

  public get length(): number {
    return this.text.length;
  }

  public slice(start: number, end: number): string {
    return this.text.slice(start, end);
  }
}

export function parseManifest(bytes: Uint8Array): ParseResult {
  const localCap = 64 * 1024 * 1024;
  if (bytes.byteLength > localCap) {
    return { ok: false, code: "MANIFEST_TOO_LARGE" };
  }
  if (
    bytes.byteLength >= 3 &&
    bytes[0] === 0xef &&
    bytes[1] === 0xbb &&
    bytes[2] === 0xbf
  ) {
    return { ok: false, code: "BYTE_ORDER_MARK" };
  }
  if (jsonNestingDepth(bytes) > MAX_JSON_NESTING) {
    return { ok: false, code: "NESTING_TOO_DEEP" };
  }
  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return { ok: false, code: "INVALID_JSON" };
  }
  const parsed = parseJsonDocument(text);
  if (!parsed.ok) {
    return parsed;
  }
  return rawToManifest(parsed.value);
}

function jsonNestingDepth(bytes: Uint8Array): number {
  let depth = 0;
  let max = 0;
  let inStr = false;
  let escape = false;
  for (const byte of bytes) {
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

function parseJsonDocument(text: string): Result<Json> {
  const cur = new Cursor(text);
  cur.skipWs();
  if (cur.atEnd) {
    return { ok: false, code: "INVALID_JSON" };
  }
  const value = parseValue(cur);
  if (!value.ok) {
    return value;
  }
  cur.skipWs();
  if (!cur.atEnd) {
    return { ok: false, code: "TRAILING_GARBAGE" };
  }
  return { ok: true, value: value.value };
}

function parseValue(cur: Cursor): Result<Json> {
  cur.skipWs();
  if (cur.atEnd) {
    return { ok: false, code: "INVALID_JSON" };
  }
  const code = cur.peek();
  if (code === 0x7b) {
    return parseObject(cur);
  }
  if (code === 0x5b) {
    return parseArray(cur);
  }
  if (code === 0x22) {
    return parseString(cur);
  }
  if (code === 0x74 || code === 0x66) {
    return parseBool(cur);
  }
  if (code === 0x6e) {
    return parseNull(cur);
  }
  if (code === 0x2d || (code >= 0x30 && code <= 0x39)) {
    return parseNumber(cur);
  }
  return { ok: false, code: "INVALID_JSON" };
}

function parseObject(cur: Cursor): Result<Json> {
  cur.consume();
  const entries = new Map<string, Json>();
  cur.skipWs();
  if (cur.peek() === 0x7d) {
    cur.consume();
    return { ok: true, value: { kind: "object", entries } };
  }
  for (;;) {
    cur.skipWs();
    const key = parseString(cur);
    if (!key.ok) {
      return key;
    }
    if (key.value.kind !== "string") {
      return { ok: false, code: "INVALID_JSON" };
    }
    if (entries.has(key.value.value)) {
      return { ok: false, code: "DUPLICATE_KEY" };
    }
    cur.skipWs();
    if (cur.peek() !== 0x3a) {
      return { ok: false, code: "INVALID_JSON" };
    }
    cur.consume();
    const value = parseValue(cur);
    if (!value.ok || value.value === undefined) {
      return value;
    }
    entries.set(key.value.value, value.value);
    cur.skipWs();
    const next = cur.peek();
    if (next === 0x2c) {
      cur.consume();
      continue;
    }
    if (next === 0x7d) {
      cur.consume();
      return { ok: true, value: { kind: "object", entries } };
    }
    return { ok: false, code: "INVALID_JSON" };
  }
}

function parseArray(cur: Cursor): Result<Json> {
  cur.consume();
  const items: Json[] = [];
  cur.skipWs();
  if (cur.peek() === 0x5d) {
    cur.consume();
    return { ok: true, value: { kind: "array", items } };
  }
  for (;;) {
    const value = parseValue(cur);
    if (!value.ok || value.value === undefined) {
      return value;
    }
    items.push(value.value);
    cur.skipWs();
    const next = cur.peek();
    if (next === 0x2c) {
      cur.consume();
      continue;
    }
    if (next === 0x5d) {
      cur.consume();
      return { ok: true, value: { kind: "array", items } };
    }
    return { ok: false, code: "INVALID_JSON" };
  }
}

function parseString(cur: Cursor): Result<Json> {
  if (cur.peek() !== 0x22) {
    return { ok: false, code: "INVALID_JSON" };
  }
  cur.consume();
  let out = "";
  while (!cur.atEnd) {
    const code = cur.consume();
    if (code === 0x22) {
      return { ok: true, value: { kind: "string", value: out } };
    }
    if (code === 0x5c) {
      const escaped = readEscape(cur);
      if (escaped === null) {
        return { ok: false, code: "INVALID_JSON" };
      }
      out += escaped;
      continue;
    }
    if (code < 0x20) {
      return { ok: false, code: "INVALID_JSON" };
    }
    out += String.fromCharCode(code);
  }
  return { ok: false, code: "INVALID_JSON" };
}

function readEscape(cur: Cursor): string | null {
  if (cur.atEnd) {
    return null;
  }
  const code = cur.consume();
  switch (code) {
    case 0x22:
      return '"';
    case 0x5c:
      return "\\";
    case 0x2f:
      return "/";
    case 0x62:
      return "\b";
    case 0x66:
      return "\f";
    case 0x6e:
      return "\n";
    case 0x72:
      return "\r";
    case 0x74:
      return "\t";
    case 0x75: {
      if (cur.pos + 4 > cur.length) {
        return null;
      }
      let value = 0;
      for (let i = 0; i < 4; i += 1) {
        const nibble = hexDigit(cur.consume());
        if (nibble < 0) {
          return null;
        }
        value = (value << 4) | nibble;
      }
      return String.fromCharCode(value);
    }
    default:
      return null;
  }
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

function parseBool(cur: Cursor): Result<Json> {
  if (cur.startsWith("true")) {
    cur.advance(4);
    return { ok: true, value: { kind: "bool", value: true } };
  }
  if (cur.startsWith("false")) {
    cur.advance(5);
    return { ok: true, value: { kind: "bool", value: false } };
  }
  return { ok: false, code: "INVALID_JSON" };
}

function parseNull(cur: Cursor): Result<Json> {
  if (cur.startsWith("null")) {
    cur.advance(4);
    return { ok: true, value: { kind: "null" } };
  }
  return { ok: false, code: "INVALID_JSON" };
}

function parseNumber(cur: Cursor): Result<Json> {
  const start = cur.pos;
  if (cur.peek() === 0x2d) {
    cur.consume();
  }
  if (cur.atEnd || cur.peek() < 0x30 || cur.peek() > 0x39) {
    return { ok: false, code: "INVALID_JSON" };
  }
  if (cur.peek() === 0x30) {
    cur.consume();
  } else {
    while (!cur.atEnd && cur.peek() >= 0x30 && cur.peek() <= 0x39) {
      cur.consume();
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
    }
  }
  const token = cur.slice(start, cur.pos);
  if (isFloat) {
    return { ok: true, value: { kind: "float" } };
  }
  if (token.startsWith("-")) {
    return { ok: true, value: { kind: "float" } };
  }
  const value = Number(token);
  if (!Number.isSafeInteger(value)) {
    return { ok: true, value: { kind: "float" } };
  }
  return { ok: true, value: { kind: "int", value } };
}

function rawToManifest(root: Json): ParseResult {
  if (root.kind !== "object") {
    return { ok: false, code: "INVALID_JSON" };
  }
  const allowed = new Set([
    "schema_version",
    "build_id",
    "client_version",
    "collected_hour",
    "description_hashes",
    "bytecode_hashes",
    "ownership",
  ]);
  for (const key of root.entries.keys()) {
    if (!allowed.has(key)) {
      return { ok: false, code: "UNKNOWN_FIELD" };
    }
  }
  const schema = takeRequired(root.entries, "schema_version");
  if (!schema.ok) {
    return schema;
  }
  if (schema.value.kind === "float") {
    return { ok: false, code: "INVALID_INTEGER" };
  }
  if (schema.value.kind !== "int") {
    return { ok: false, code: "INVALID_INTEGER" };
  }
  if (schema.value.value !== SCHEMA_VERSION) {
    return { ok: false, code: "UNSUPPORTED_SCHEMA_VERSION" };
  }
  const build = takeRequired(root.entries, "build_id");
  if (!build.ok) {
    return build;
  }
  const buildId = parseBuildId(build.value);
  if (!buildId.ok) {
    return buildId;
  }
  const client = takeRequired(root.entries, "client_version");
  if (!client.ok) {
    return client;
  }
  const clientVersion = parseClientVersion(client.value);
  if (!clientVersion.ok) {
    return clientVersion;
  }
  const hour = takeRequired(root.entries, "collected_hour");
  if (!hour.ok) {
    return hour;
  }
  const collectedHour = parseUtcHour(hour.value);
  if (!collectedHour.ok) {
    return collectedHour;
  }
  const desc = takeRequired(root.entries, "description_hashes");
  if (!desc.ok) {
    return desc;
  }
  const description = parseHashList(desc.value, MAX_DESCRIPTION_HASHES);
  if (!description.ok) {
    return description;
  }
  const code = takeRequired(root.entries, "bytecode_hashes");
  if (!code.ok) {
    return code;
  }
  const bytecode = parseHashList(code.value, MAX_BYTECODE_HASHES);
  if (!bytecode.ok) {
    return bytecode;
  }
  const ownershipRaw = takeRequired(root.entries, "ownership");
  if (!ownershipRaw.ok) {
    return ownershipRaw;
  }
  const ownership = parseOwnership(ownershipRaw.value);
  if (!ownership.ok) {
    return ownership;
  }
  return {
    ok: true,
    manifest: {
      schema_version: SCHEMA_VERSION,
      build_id: buildId.value,
      client_version: clientVersion.value,
      collected_hour: collectedHour.value,
      description_hashes: description.value,
      bytecode_hashes: bytecode.value,
      ownership: ownership.value,
    },
  };
}

function takeRequired(entries: ReadonlyMap<string, Json>, key: string): Result<Json> {
  const value = entries.get(key);
  if (value === undefined) {
    return { ok: false, code: "MISSING_FIELD" };
  }
  return { ok: true, value };
}

function parseBuildId(value: Json): Result<UploadManifest["build_id"]> {
  if (value.kind !== "object") {
    return { ok: false, code: "INVALID_JSON" };
  }
  for (const key of value.entries.keys()) {
    if (key !== "game_id" && key !== "version" && key !== "exe_hash") {
      return { ok: false, code: "UNKNOWN_FIELD" };
    }
  }
  const game = takeStringField(value.entries, "game_id");
  if (!game.ok) {
    return game;
  }
  const version = takeStringField(value.entries, "version");
  if (!version.ok) {
    return version;
  }
  const exe = takeStringField(value.entries, "exe_hash");
  if (!exe.ok) {
    return exe;
  }
  if (!isSafeIdent(game.value) || !isSafeIdent(version.value)) {
    return { ok: false, code: "INVALID_BUILD_ID" };
  }
  const exeHash = parseHex(exe.value);
  if (!exeHash.ok) {
    return exeHash;
  }
  return {
    ok: true,
    value: { game_id: game.value, version: version.value, exe_hash: exeHash.value },
  };
}

function takeStringField(entries: ReadonlyMap<string, Json>, key: string): Result<string> {
  const value = entries.get(key);
  if (value === undefined) {
    return { ok: false, code: "MISSING_FIELD" };
  }
  if (value.kind !== "string") {
    return { ok: false, code: "INVALID_JSON" };
  }
  return { ok: true, value: value.value };
}

function parseClientVersion(value: Json): Result<string> {
  if (value.kind !== "string") {
    return { ok: false, code: "INVALID_CLIENT_VERSION" };
  }
  const text = value.value;
  if (text.length === 0 || text.length > 32) {
    return { ok: false, code: "INVALID_CLIENT_VERSION" };
  }
  const parts = text.split(".");
  if (parts.length !== 3) {
    return { ok: false, code: "INVALID_CLIENT_VERSION" };
  }
  for (const part of parts) {
    if (part.length === 0 || ![...part].every((ch) => ch >= "0" && ch <= "9")) {
      return { ok: false, code: "INVALID_CLIENT_VERSION" };
    }
  }
  return { ok: true, value: text };
}

function parseUtcHour(value: Json): Result<string> {
  if (value.kind !== "string") {
    return { ok: false, code: "INVALID_TIMESTAMP" };
  }
  const text = value.value;
  if (text.length !== 20) {
    return { ok: false, code: "INVALID_TIMESTAMP" };
  }
  const match = /^([0-9]{4})-([0-9]{2})-([0-9]{2})T([0-9]{2}):00:00Z$/.exec(text);
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
  return { ok: true, value: text };
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

function parseHashList(value: Json, max: number): Result<Sha256Hex[]> {
  if (value.kind !== "array") {
    return { ok: false, code: "INVALID_JSON" };
  }
  if (value.items.length > max) {
    return { ok: false, code: "TOO_MANY_HASHES" };
  }
  const hashes: Sha256Hex[] = [];
  for (const item of value.items) {
    if (item.kind !== "string") {
      return { ok: false, code: "INVALID_JSON" };
    }
    const hex = parseHex(item.value);
    if (!hex.ok) {
      return hex;
    }
    hashes.push(hex.value);
  }
  for (let i = 1; i < hashes.length; i += 1) {
    const prev = hashes[i - 1];
    const cur = hashes[i];
    if (prev === undefined || cur === undefined || prev >= cur) {
      return { ok: false, code: "UNSORTED_OR_DUPLICATE_HASHES" };
    }
  }
  return { ok: true, value: hashes };
}

function parseHex(text: string): Result<Sha256Hex> {
  if (text.length !== 64) {
    return { ok: false, code: "WRONG_HEX_LENGTH" };
  }
  for (let i = 0; i < text.length; i += 1) {
    const ch = text.charCodeAt(i);
    const ok =
      (ch >= 0x30 && ch <= 0x39) || (ch >= 0x61 && ch <= 0x66);
    if (!ok) {
      return { ok: false, code: "MALFORMED_HEX" };
    }
  }
  return { ok: true, value: text };
}

function parseOwnership(value: Json): Result<OwnershipProof> {
  if (value.kind !== "object") {
    return { ok: false, code: "INVALID_JSON" };
  }
  for (const key of value.entries.keys()) {
    if (
      key !== "kind" &&
      key !== "reason" &&
      key !== "challenge" &&
      key !== "response"
    ) {
      return { ok: false, code: "UNKNOWN_FIELD" };
    }
  }
  const kindRaw = takeStringField(value.entries, "kind");
  if (!kindRaw.ok) {
    return kindRaw;
  }
  if (kindRaw.value === "unchallenged") {
    if (value.entries.has("challenge") || value.entries.has("response")) {
      return { ok: false, code: "UNKNOWN_FIELD" };
    }
    const reason = takeStringField(value.entries, "reason");
    if (!reason.ok) {
      return reason;
    }
    if (reason.value !== "no_server_challenge") {
      return { ok: false, code: "UNKNOWN_OWNERSHIP_REASON" };
    }
    return {
      ok: true,
      value: { kind: "unchallenged", reason: "no_server_challenge" },
    };
  }
  if (kindRaw.value === "exe_binding_v1") {
    if (value.entries.has("reason")) {
      return { ok: false, code: "UNKNOWN_FIELD" };
    }
    const challenge = takeStringField(value.entries, "challenge");
    if (!challenge.ok) {
      return challenge;
    }
    const response = takeStringField(value.entries, "response");
    if (!response.ok) {
      return response;
    }
    const ch = parseHex(challenge.value);
    if (!ch.ok) {
      return ch;
    }
    const rs = parseHex(response.value);
    if (!rs.ok) {
      return rs;
    }
    return {
      ok: true,
      value: { kind: "exe_binding_v1", challenge: ch.value, response: rs.value },
    };
  }
  return { ok: false, code: "UNKNOWN_OWNERSHIP_KIND" };
}
