// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { test } from "node:test";

import { readUtf8 } from "./helpers.js";

const LOGPUSH_ALLOWLIST = [
  "EventType",
  "Outcome",
  "ScriptName",
  "Logs",
  "CPUTimeMs",
  "WallTimeMs",
  "EventTimestampMs",
];

test("R2 lifecycle rule expires objects after 90 days", () => {
  const text = readUtf8("worker/config/r2-lifecycle.json");
  const config = JSON.parse(text) as {
    rules: { expiration: { days: number } }[];
  };
  assert.equal(config.rules.length, 1);
  assert.equal(config.rules[0]?.expiration.days, 90);
  assert.match(text, /90/);
});

test("Logpush field list is an exact low-risk allowlist", () => {
  const text = readUtf8("worker/config/logpush-fields.json");
  const config = JSON.parse(text) as {
    dataset: string;
    output_options: { field_names: string[] };
  };
  assert.equal(config.dataset, "workers_trace_events");
  assert.deepEqual(config.output_options.field_names, LOGPUSH_ALLOWLIST);
  assert.equal(config.output_options.field_names.includes("Event"), false);
  assert.equal(config.output_options.field_names.includes("Exceptions"), false);
});

test("wrangler config is placeholders and disables Workers Logs", () => {
  const text = readUtf8("worker/wrangler.toml");
  assert.match(text, /NOT DEPLOYED/);
  assert.match(text, /PLACEHOLDER/);
  assert.match(text, /00000000000000000000000000000000/);
  assert.match(text, /\[observability\]/);
  assert.match(text, /enabled = false/);
  assert.doesNotMatch(text, /CF-Connecting-IP/);
});
