// SPDX-License-Identifier: MIT

import type { ErrorCode } from "./errors.js";
import type { UploadManifest } from "./types.js";

const HEX64 = /^[0-9a-f]{64}$/;

export function buildIdComponent(version: string, exeHash: string): string {
  return `${version}--${exeHash}`;
}

export function objectKey(
  manifest: UploadManifest,
  canonicalSha256: string,
): { ok: true; key: string; prefix: string } | { ok: false; code: ErrorCode } {
  const game = manifest.build_id.game_id;
  const version = manifest.build_id.version;
  const exe = manifest.build_id.exe_hash;
  if (!isSafeIdent(game) || isDotComponent(game)) {
    return { ok: false, code: "INVALID_BUILD_ID" };
  }
  if (!isSafeIdent(version) || isDotComponent(version)) {
    return { ok: false, code: "INVALID_BUILD_ID" };
  }
  if (!HEX64.test(exe) || !HEX64.test(canonicalSha256)) {
    return { ok: false, code: "MALFORMED_HEX" };
  }
  const buildId = buildIdComponent(version, exe);
  if (buildId.includes("/") || buildId.includes("\\") || isDotComponent(buildId)) {
    return { ok: false, code: "INVALID_BUILD_ID" };
  }
  const prefix = `${game}/${buildId}/`;
  return { ok: true, key: `${prefix}${canonicalSha256}.json`, prefix };
}

export function isSafeIdent(text: string): boolean {
  if (text.length < 1 || text.length > 128) {
    return false;
  }
  for (let i = 0; i < text.length; i += 1) {
    const code = text.charCodeAt(i);
    if (
      code === 0x2f ||
      code === 0x5c ||
      code === 0x3a ||
      code === 0 ||
      code <= 0x20 ||
      code > 0x7e
    ) {
      return false;
    }
  }
  return true;
}

function isDotComponent(text: string): boolean {
  return text === "." || text === "..";
}
