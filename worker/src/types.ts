// SPDX-License-Identifier: MIT

export type SchemaVersion = 1;

export type Sha256Hex = string;

export interface BuildId {
  readonly game_id: string;
  readonly version: string;
  readonly exe_hash: Sha256Hex;
}

export type OwnershipProof =
  | { readonly kind: "unchallenged"; readonly reason: "no_server_challenge" }
  | {
      readonly kind: "exe_binding_v1";
      readonly challenge: Sha256Hex;
      readonly response: Sha256Hex;
    };

export interface UploadManifest {
  readonly schema_version: SchemaVersion;
  readonly build_id: BuildId;
  readonly client_version: string;
  readonly collected_hour: string;
  readonly description_hashes: readonly Sha256Hex[];
  readonly bytecode_hashes: readonly Sha256Hex[];
  readonly ownership: OwnershipProof;
}

export interface UploadReceipt {
  readonly accepted: true;
  readonly idempotent: boolean;
  readonly ownership_state: "UNVERIFIED";
  readonly schema_version: SchemaVersion;
  readonly build_id: BuildId;
  readonly collected_hour: string;
  readonly description_hash_count: number;
  readonly bytecode_hash_count: number;
  readonly canonical_sha256: Sha256Hex;
  readonly object_key: string;
}

export interface StagingStore {
  head(key: string): Promise<boolean>;
  put(key: string, bytes: Uint8Array): Promise<void>;
  countPrefix(prefix: string, limit: number): Promise<number>;
}

export interface RateGate {
  allow(): Promise<boolean>;
}

export interface InjectedFetch {
  (
    input: string,
    init: {
      method: string;
      headers: Record<string, string>;
      body: string;
      signal?: AbortSignal;
    },
  ): Promise<{
    readonly ok: boolean;
    json(): Promise<unknown>;
  }>;
}

export interface TurnstileClient {
  verify(turnstileToken: string): Promise<boolean>;
}

export interface HandlerDeps {
  readonly store: StagingStore;
  readonly rate: RateGate;
  readonly turnstile: TurnstileClient;
  readonly perBuildCap: number;
  readonly wireMaxBytes: number;
}

export interface R2ObjectBody {
  arrayBuffer(): Promise<ArrayBuffer>;
}

export interface R2ListResult {
  readonly objects: readonly { readonly key: string }[];
  readonly truncated: boolean;
  readonly cursor?: string;
}

export interface R2Bucket {
  head(key: string): Promise<{ readonly key: string } | null>;
  get(key: string): Promise<R2ObjectBody | null>;
  put(key: string, value: ArrayBuffer | Uint8Array): Promise<unknown>;
  list(options: {
    prefix: string;
    limit?: number;
    cursor?: string;
  }): Promise<R2ListResult>;
}

export interface RateLimitBinding {
  limit(options: { key: string }): Promise<{ success: boolean }>;
}

export interface WorkerEnv {
  readonly STAGING_BUCKET: R2Bucket;
  readonly UPLOAD_RATE_LIMITER: RateLimitBinding;
  readonly TURNSTILE_SECRET: string;
  readonly TURNSTILE_SITEVERIFY_URL: string;
  readonly TURNSTILE_EXPECTED_HOSTNAME?: string;
  readonly TURNSTILE_EXPECTED_ACTION?: string;
  readonly PER_BUILD_UPLOAD_CAP?: string;
}
