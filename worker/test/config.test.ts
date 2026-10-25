// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { test } from "node:test";

import { readUtf8 } from "./helpers.js";

test("R2 lifecycle rule expires objects after 90 days", () => {
  const text = readUtf8("worker/config/r2-lifecycle.json");
  const config = JSON.parse(text) as {
    rules: { expiration: { days: number } }[];
  };
  assert.equal(config.rules.length, 1);
  assert.equal(config.rules[0]?.expiration.days, 90);
  assert.match(text, /90/);
});

test("Logpush field list omits ClientIP", () => {
  const text = readUtf8("worker/config/logpush-fields.json");
  assert.equal(/clientip/i.test(text), false, "Logpush template must omit ClientIP");
  const config = JSON.parse(text) as {
    output_options: { field_names: string[] };
  };
  for (const name of config.output_options.field_names) {
    assert.equal(/clientip/i.test(name), false, `field ${name} must not be ClientIP`);
  }
});

test("wrangler config is placeholders and marked not deployed", () => {
  const text = readUtf8("worker/wrangler.toml");
  assert.match(text, /NOT DEPLOYED/);
  assert.match(text, /PLACEHOLDER/);
  assert.match(text, /00000000000000000000000000000000/);
  assert.doesNotMatch(text, /CF-Connecting-IP/);
});
