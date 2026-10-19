import { describe, expect, it } from "vitest";
import { displayVarValue, isSensitiveKey, maskJsonForDisplay, maskValue, redactText } from "@/lib/mask";

describe("isSensitiveKey", () => {
  it("detects common credential field names", () => {
    for (const key of [
      "API_KEY",
      "apiKey",
      "GITHUB_TOKEN",
      "secret",
      "PASSWORD",
      "Authorization",
      "private_key",
    ]) {
      expect(isSensitiveKey(key)).toBe(true);
    }
  });

  it("does not flag ordinary configuration", () => {
    for (const key of ["PATH", "NODE_ENV", "WORKSPACE", "LOG_LEVEL"]) {
      expect(isSensitiveKey(key)).toBe(false);
    }
  });
});

describe("maskValue", () => {
  it("keeps short values fully hidden", () => {
    expect(maskValue("abc")).toHaveLength(6);
  });

  it("reveals only a small prefix and suffix", () => {
    const masked = maskValue("sk-abcdefghijklmnop");
    expect(masked.startsWith("sk-")).toBe(true);
    expect(masked.endsWith("op")).toBe(true);
    expect(masked).not.toContain("defghijklmn");
  });

  it("passes environment and file references through untouched", () => {
    expect(maskValue("{env:GITHUB_TOKEN}")).toBe("{env:GITHUB_TOKEN}");
    expect(maskValue("{file:~/.secrets/key}")).toBe("{file:~/.secrets/key}");
  });
});

describe("displayVarValue", () => {
  it("never prints secret values", () => {
    const output = displayVarValue("GITHUB_TOKEN", "ghp_supersecretvalue", "secret");
    expect(output).not.toContain("ghp_supersecretvalue");
    expect(output).toContain("credential store");
  });

  it("masks sensitive literal values", () => {
    const output = displayVarValue("API_KEY", "1234567890abcdef");
    expect(output).not.toContain("1234567890abcdef");
  });

  it("shows ordinary values", () => {
    expect(displayVarValue("WORKSPACE", "C:/work")).toBe("C:/work");
  });
});

describe("maskJsonForDisplay", () => {
  it("masks nested credential fields but keeps structure", () => {
    const masked = maskJsonForDisplay({
      command: "npx",
      env: { GITHUB_TOKEN: "ghp_supersecretvalue", LOG_LEVEL: "debug" },
      nested: [{ headers: { Authorization: "Bearer abcdefghijklmn" } }],
    }) as any;

    expect(masked.command).toBe("npx");
    expect(masked.env.LOG_LEVEL).toBe("debug");
    expect(masked.env.GITHUB_TOKEN).not.toContain("ghp_supersecretvalue");
    expect(masked.nested[0].headers.Authorization).not.toContain("abcdefghijklmn");
  });
});

describe("redactText", () => {
  it("redacts bearer tokens and JSON credential values", () => {
    const redacted = redactText('{"apiKey": "abcdef123456", "note": "Bearer aaaabbbbccccdddd"}');
    expect(redacted).not.toContain("abcdef123456");
    expect(redacted).not.toContain("aaaabbbbccccdddd");
    expect(redacted).toContain('"note"');
  });
});
