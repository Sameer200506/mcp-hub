import { describe, expect, it } from "vitest";
import { highlightJson } from "@/components/ui/data-display";

describe("highlightJson", () => {
  it("escapes HTML so config content cannot inject markup", () => {
    const html = highlightJson('{"cmd": "<script>alert(1)</script>"}');
    expect(html).not.toContain("<script>");
    expect(html).toContain("&lt;script&gt;");
  });

  it("wraps keys, strings, booleans, nulls and numbers", () => {
    const html = highlightJson('{"a": "b", "n": 1.5, "t": true, "z": null}');
    expect(html).toContain('class="tok-key"');
    expect(html).toContain('class="tok-string"');
    expect(html).toContain('class="tok-number"');
    expect(html).toContain('class="tok-bool"');
    expect(html).toContain('class="tok-null"');
  });

  it("handles escaped quotes inside strings", () => {
    const html = highlightJson('{"a": "quote \\" here"}');
    expect(html).toContain('class="tok-string"');
    expect(html).toContain("quote");
  });
});
