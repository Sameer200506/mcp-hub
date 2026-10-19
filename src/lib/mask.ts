/**
 * Client-side display masking. The Rust backend already redacts secrets in
 * previews and logs; this is defence in depth for anything rendered in the UI.
 * It never *decrypts* or reveals a value.
 */

const SENSITIVE_KEY_PATTERN =
  /(api[-_]?key|token|secret|password|passwd|credential|authorization|auth|bearer|private[-_]?key|session|cookie)/i;

export function isSensitiveKey(key: string): boolean {
  return SENSITIVE_KEY_PATTERN.test(key);
}

/** Mask a value for display. Short values are fully hidden. */
export function maskValue(value: string, visiblePrefix = 3, visibleSuffix = 2): string {
  if (!value) return "";
  const isEnvRef = /^\{env:[^}]+\}$/.test(value);
  const isFileRef = /^\{file:[^}]+\}$/.test(value);
  if (isEnvRef || isFileRef) return value;
  if (value.length <= visiblePrefix + visibleSuffix) return "•".repeat(Math.max(value.length, 6));
  return `${value.slice(0, visiblePrefix)}${"•".repeat(8)}${value.slice(-visibleSuffix)}`;
}

export function displayVarValue(key: string, value: string, kind?: string): string {
  if (kind === "secret") return "•••••••• (stored in OS credential store)";
  if (isSensitiveKey(key)) return maskValue(value);
  return value;
}

/** Deep-mask a JSON-ish object for display purposes. */
export function maskJsonForDisplay(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(maskJsonForDisplay);
  if (value && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(value as Record<string, unknown>)) {
      if (typeof item === "string" && isSensitiveKey(key)) {
        out[key] = maskValue(item);
      } else {
        out[key] = maskJsonForDisplay(item);
      }
    }
    return out;
  }
  return value;
}

/** Mask sensitive-looking strings such as bearer tokens inside URLs/headers. */
export function redactText(text: string): string {
  return text
    .replace(/(Bearer\s+)[A-Za-z0-9._~+/=-]{8,}/gi, "$1••••••••")
    .replace(
      /("(?:api[-_]?key|token|secret|password|authorization)"\s*:\s*")([^"]{4,})(")/gi,
      "$1••••••••$3",
    );
}
