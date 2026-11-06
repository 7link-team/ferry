import DOMPurify from "dompurify";

/** Preserve source rich-text formatting while removing active document content. */
export function sanitizeRichText(value: string): string {
  return DOMPurify.sanitize(value, {
    USE_PROFILES: { html: true },
    FORBID_TAGS: ["style", "form", "input", "button", "textarea", "select", "iframe", "object", "embed"],
    FORBID_ATTR: ["srcdoc", "autofocus", "contenteditable"],
  });
}
