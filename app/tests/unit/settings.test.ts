import { expect, test } from "vitest";
import type { Settings } from "../../src/api/types";
import { parseLimit, validateSettings } from "../../src/lib/settings";

const base: Settings = {
  image: "img",
  model: null,
  provider_env: {},
  default_network: { mode: "none" },
  permission_mode: "bypassPermissions",
  limits: { memory_mb: 8192, cpus: null, pids: 4096 },
};

test("validateSettings mirrors the Rust rules", () => {
  expect(validateSettings(base)).toBeNull();
  expect(validateSettings({ ...base, image: " " })).toContain("image");
  expect(validateSettings({ ...base, permission_mode: "yolo" as never })).toContain("permission");
  expect(validateSettings({ ...base, limits: { memory_mb: 255, cpus: null, pids: null } })).toContain("memory");
  expect(validateSettings({ ...base, limits: { memory_mb: 256.5, cpus: null, pids: null } })).toContain("memory");
  expect(validateSettings({ ...base, limits: { memory_mb: null, cpus: 0.05, pids: null } })).toContain("CPU");
  expect(validateSettings({ ...base, limits: { memory_mb: null, cpus: NaN, pids: null } })).toContain("CPU");
  expect(validateSettings({ ...base, limits: { memory_mb: null, cpus: 1.5, pids: 63 } })).toContain("process");
  expect(validateSettings({ ...base, limits: { memory_mb: null, cpus: null, pids: null } })).toBeNull();
});

test("parseLimit", () => {
  expect(parseLimit("")).toBeNull();
  expect(parseLimit("  ")).toBeNull();
  expect(parseLimit("512")).toBe(512);
  expect(parseLimit("1.5")).toBe(1.5);
  expect(parseLimit("lots")).toBeNaN();
});
