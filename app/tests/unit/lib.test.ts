import { expect, test } from "vitest";
import { errorMessage, formatCost, relativeTime, reliability, shortId } from "../../src/lib/format";
import { describePolicy, parseHosts } from "../../src/lib/network";

test("parseHosts normalises and validates", () => {
  expect(parseHosts("")).toEqual({ hosts: [], invalid: [] });
  expect(parseHosts("registry.npmjs.org, https://PyPI.org/simple\n*.githubusercontent.com  pypi.org:443")).toEqual({
    hosts: ["registry.npmjs.org", "pypi.org", "*.githubusercontent.com"],
    invalid: [],
  });
  expect(parseHosts("bad_host -x- a..b *").invalid).toEqual(["bad_host", "-x-", "a..b"]);
  expect(parseHosts("*").hosts).toEqual(["*"]);
});

test("describePolicy", () => {
  expect(describePolicy({ mode: "none" })).toContain("No network");
  expect(describePolicy({ mode: "full" })).toContain("Full");
  expect(describePolicy({ mode: "allowlist", hosts: [] })).toContain("empty");
  expect(describePolicy({ mode: "allowlist", hosts: ["a.com", "b.com"] })).toBe("Allowlist: a.com, b.com");
});

test("format helpers", () => {
  expect(shortId("abcdef123456")).toBe("abcdef1");
  expect(relativeTime(1000, 1000)).toBe("just now");
  expect(relativeTime(1000, 1000 - 50)).toBe("just now"); // clock skew into the future
  expect(relativeTime(0, 600)).toBe("10m ago");
  expect(relativeTime(0, 7200)).toBe("2h ago");
  expect(relativeTime(0, 86400 * 3)).toBe("3d ago");
  expect(relativeTime(0, 86400 * 100)).toBe("1970-01-01");
  expect(formatCost(null)).toBe("");
  expect(formatCost(NaN)).toBe("");
  expect(formatCost(0.001)).toBe("<$0.01");
  expect(formatCost(1.234)).toBe("$1.23");
  expect(errorMessage("x")).toBe("x");
  expect(errorMessage(new Error("y"))).toBe("y");
  expect(errorMessage({ message: "z" })).toBe("z");
  expect(errorMessage(42)).toBe("42");
  expect(reliability({ successes: 0, failures: 0, negative: 0 })).toBe(0.5);
  expect(reliability({ successes: 8, failures: 0, negative: 0 })).toBe(0.9);
});
