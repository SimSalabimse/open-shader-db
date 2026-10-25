// SPDX-License-Identifier: MIT

import type { R2Bucket, StagingStore } from "./types.js";

export class MemoryStore implements StagingStore {
  private readonly objects = new Map<string, Uint8Array>();

  async get(key: string): Promise<Uint8Array | null> {
    const found = this.objects.get(key);
    if (found === undefined) {
      return null;
    }
    return new Uint8Array(found);
  }

  async putIfAbsent(key: string, bytes: Uint8Array): Promise<"created" | "exists"> {
    if (this.objects.has(key)) {
      return "exists";
    }
    this.objects.set(key, new Uint8Array(bytes));
    return "created";
  }

  async countPrefix(prefix: string, limit: number): Promise<number> {
    let count = 0;
    for (const key of this.objects.keys()) {
      if (key.startsWith(prefix)) {
        count += 1;
        if (count >= limit) {
          return count;
        }
      }
    }
    return count;
  }
}

export class R2StagingStore implements StagingStore {
  public constructor(private readonly bucket: R2Bucket) {}

  async get(key: string): Promise<Uint8Array | null> {
    const object = await this.bucket.get(key);
    if (object === null) {
      return null;
    }
    return new Uint8Array(await object.arrayBuffer());
  }

  async putIfAbsent(key: string, bytes: Uint8Array): Promise<"created" | "exists"> {
    const existing = await this.bucket.get(key);
    if (existing !== null) {
      return "exists";
    }
    await this.bucket.put(key, bytes);
    return "created";
  }

  async countPrefix(prefix: string, limit: number): Promise<number> {
    let count = 0;
    let cursor: string | undefined;
    do {
      const remaining = limit - count;
      const page = await this.bucket.list({
        prefix,
        limit: remaining,
        ...(cursor === undefined ? {} : { cursor }),
      });
      count += page.objects.length;
      if (count >= limit || !page.truncated) {
        return count;
      }
      cursor = page.cursor;
    } while (cursor !== undefined);
    return count;
  }
}
