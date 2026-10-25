// SPDX-License-Identifier: MIT

import type { UploadManifest } from "./types.js";

export function canonicalize(manifest: UploadManifest): Uint8Array {
  const ownership =
    manifest.ownership.kind === "unchallenged"
      ? { kind: "unchallenged", reason: "no_server_challenge" }
      : {
          challenge: manifest.ownership.challenge,
          kind: "exe_binding_v1",
          response: manifest.ownership.response,
        };
  const value = {
    build_id: {
      exe_hash: manifest.build_id.exe_hash,
      game_id: manifest.build_id.game_id,
      version: manifest.build_id.version,
    },
    bytecode_hashes: [...manifest.bytecode_hashes],
    client_version: manifest.client_version,
    collected_hour: manifest.collected_hour,
    description_hashes: [...manifest.description_hashes],
    ownership,
    schema_version: manifest.schema_version,
  };
  return new TextEncoder().encode(JSON.stringify(value));
}

export async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return hexEncode(new Uint8Array(digest));
}

export function hexEncode(bytes: Uint8Array): string {
  let out = "";
  for (const byte of bytes) {
    out += (byte >> 4).toString(16);
    out += (byte & 0x0f).toString(16);
  }
  return out;
}
