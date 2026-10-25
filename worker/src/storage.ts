// SPDX-License-Identifier: MIT

import type { R2Bucket, StagingStore } from "./types.js";

export class MemoryStore implements StagingStore {
  private readonly objects = new Map<string, Uint8Array>();

  async head(key: string): Promise<boolean> {
    return this.objects.has(key);
  }

  async read(key: string): Promise<Uint8Array | null> {
    const found = this.objects.get(key);
    if (found === undefined) {
      return null;
    }
    return new Uint8Array(found);
  }

  async put(key: string, bytes: Uint8Array): Promise<void> {
    this.objects.set(key, new Uint8Array(bytes));
  }

  async putIfAbsent(key: string, bytes: Uint8Array): Promise<"created" | "exists"> {
    if (await this.head(key)) {
      return "exists";
    }
    await this.put(key, bytes);
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

  async head(key: string): Promise<boolean> {
    return (await this.bucket.head(key)) !== null;
  }

  async put(key: string, bytes: Uint8Array): Promise<void> {
    await this.bucket.put(key, bytes);
  }

  async putIfAbsent(key: string, bytes: Uint8Array): Promise<"created" | "exists"> {
    if (await this.head(key)) {
      return "exists";
    }
    await this.put(key, bytes);
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
