// SPDX-License-Identifier: MIT

import { declaredBodyTooLarge, readBodyCapped } from "./body.js";
import { sha256Hex } from "./canonical.js";
import { errorBody, httpStatus, type ErrorCode } from "./errors.js";
import { objectKey } from "./keys.js";
import { TURNSTILE_TOKEN_MAX_CHARS, WIRE_MAX_JSON_BYTES } from "./limits.js";
import { logJson } from "./log.js";
import { parseManifest } from "./parse.js";
import { isJsonContentType, readAllowedHeader } from "./privacy.js";
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
  if (!isJsonContentType(readAllowedHeader(request.headers, "Content-Type"))) {
    return errorResponse("UNSUPPORTED_MEDIA_TYPE");
  }
  if (declaredBodyTooLarge(request, deps.wireMaxBytes)) {
    return errorResponse("BODY_TOO_LARGE");
  }
  const turnstileToken = readAllowedHeader(request.headers, "X-Turnstile-Token");
  if (turnstileToken === null || turnstileToken.length === 0) {
    return errorResponse("TURNSTILE_MISSING");
  }
  if (turnstileToken.length > TURNSTILE_TOKEN_MAX_CHARS) {
    return errorResponse("TURNSTILE_FAILED");
  }
  const verified = await deps.turnstile.verify(turnstileToken);
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
  return storeManifest(
    parsed.manifest,
    parsed.storedBytes,
    parsed.descriptionHashCount,
    parsed.bytecodeHashCount,
    deps,
  );
}

async function storeManifest(
  manifest: UploadManifest,
  storedBytes: Uint8Array,
  descriptionHashCount: number,
  bytecodeHashCount: number,
  deps: HandlerDeps,
): Promise<Response> {
  const digest = await sha256Hex(storedBytes);
  const key = objectKey(manifest, digest);
  if (!key.ok) {
    return errorResponse(key.code);
  }
  if (await deps.store.head(key.key)) {
    logOutcome("accepted", { prefix: key.prefix, idempotent: true });
    return jsonResponse(
      200,
      receipt(manifest, digest, key.key, true, descriptionHashCount, bytecodeHashCount),
    );
  }
  const count = await deps.store.countPrefix(key.prefix, deps.perBuildCap);
  if (count >= deps.perBuildCap) {
    return errorResponse("BUILD_CAP_EXCEEDED");
  }
  await deps.store.put(key.key, storedBytes);
  logOutcome("accepted", {
    prefix: key.prefix,
    idempotent: false,
    description_hash_count: descriptionHashCount,
    bytecode_hash_count: bytecodeHashCount,
  });
  return jsonResponse(
    200,
    receipt(manifest, digest, key.key, false, descriptionHashCount, bytecodeHashCount),
  );
}

function receipt(
  manifest: UploadManifest,
  canonicalSha256: string,
  objectKeyValue: string,
  idempotent: boolean,
  descriptionHashCount: number,
  bytecodeHashCount: number,
): UploadReceipt {
  return {
    accepted: true,
    idempotent,
    ownership_state: "UNVERIFIED",
    schema_version: manifest.schema_version,
    build_id: manifest.build_id,
    collected_hour: manifest.collected_hour,
    description_hash_count: descriptionHashCount,
    bytecode_hash_count: bytecodeHashCount,
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
  logJson({
    event: "upload",
    code,
    ...extra,
  });
}

export const WIRE_CAP = WIRE_MAX_JSON_BYTES;
