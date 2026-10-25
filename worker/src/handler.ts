// SPDX-License-Identifier: MIT

import { readBodyCapped } from "./body.js";
import { canonicalize, sha256Hex } from "./canonical.js";
import { errorBody, httpStatus, type ErrorCode } from "./errors.js";
import { objectKey } from "./keys.js";
import { WIRE_MAX_JSON_BYTES } from "./limits.js";
import { parseManifest } from "./parse.js";
import { readAllowedHeader } from "./privacy.js";
import type { HandlerDeps, UploadManifest, UploadReceipt } from "./types.js";

const UPLOAD_PATH = "/v1/upload";

export async function handleUpload(
  request: Request,
  deps: HandlerDeps,
): Promise<Response> {
  const url = new URL(request.url);
  if (url.pathname !== UPLOAD_PATH) {
    return errorResponse("NOT_FOUND");
  }
  if (request.method !== "POST") {
    return errorResponse("METHOD_NOT_ALLOWED");
  }
  if (!(await deps.rate.allow())) {
    return errorResponse("RATE_LIMITED");
  }
  const token = readAllowedHeader(request.headers, "X-Turnstile-Token");
  if (token === null || token.length === 0) {
    return errorResponse("TURNSTILE_MISSING");
  }
  const verified = await deps.turnstile.verify(token);
  if (!verified) {
    return errorResponse("TURNSTILE_FAILED");
  }
  const body = await readBodyCapped(request, deps.wireMaxBytes);
  if (!body.ok) {
    return errorResponse(body.code);
  }
  const parsed = parseManifest(body.bytes);
  if (!parsed.ok) {
    return errorResponse(parsed.code);
  }
  return storeManifest(parsed.manifest, deps);
}

async function storeManifest(
  manifest: UploadManifest,
  deps: HandlerDeps,
): Promise<Response> {
  const canonical = canonicalize(manifest);
  const digest = await sha256Hex(canonical);
  const key = objectKey(manifest, digest);
  if (!key.ok) {
    return errorResponse(key.code);
  }
  const existing = await deps.store.get(key.key);
  if (existing !== null) {
    logOutcome("accepted", { prefix: key.prefix, idempotent: true });
    return jsonResponse(200, receipt(manifest, digest, key.key, true));
  }
  const count = await deps.store.countPrefix(key.prefix, deps.perBuildCap);
  if (count >= deps.perBuildCap) {
    return errorResponse("BUILD_CAP_EXCEEDED");
  }
  const put = await deps.store.putIfAbsent(key.key, canonical);
  const idempotent = put === "exists";
  logOutcome("accepted", {
    prefix: key.prefix,
    idempotent,
    description_hash_count: manifest.description_hashes.length,
    bytecode_hash_count: manifest.bytecode_hashes.length,
  });
  return jsonResponse(200, receipt(manifest, digest, key.key, idempotent));
}

function receipt(
  manifest: UploadManifest,
  canonicalSha256: string,
  objectKeyValue: string,
  idempotent: boolean,
): UploadReceipt {
  return {
    accepted: true,
    idempotent,
    ownership_state: "UNVERIFIED",
    schema_version: manifest.schema_version,
    build_id: manifest.build_id,
    collected_hour: manifest.collected_hour,
    description_hash_count: manifest.description_hashes.length,
    bytecode_hash_count: manifest.bytecode_hashes.length,
    canonical_sha256: canonicalSha256,
    object_key: objectKeyValue,
  };
}

function errorResponse(code: ErrorCode): Response {
  logOutcome(code, {});
  return jsonResponse(httpStatus(code), errorBody(code));
}

function jsonResponse(status: number, value: UploadReceipt | ReturnType<typeof errorBody>): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json; charset=utf-8" },
  });
}

function logOutcome(
  code: string,
  extra: {
    prefix?: string;
    idempotent?: boolean;
    description_hash_count?: number;
    bytecode_hash_count?: number;
  },
): void {
  console.log(
    JSON.stringify({
      event: "upload",
      code,
      ...extra,
    }),
  );
}

export const WIRE_CAP = WIRE_MAX_JSON_BYTES;
