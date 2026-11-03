export type TextEncoding = "utf-8" | "utf-8-bom" | "utf-16le" | "utf-16be" | "gb18030" | "unknown";
export function decodeTextFile(bytes: Uint8Array): { value: string; encoding: TextEncoding; readOnly: boolean } {
  let encoding: TextEncoding = "utf-8";
  let value: string;
  if (bytes[0] === 0xff && bytes[1] === 0xfe || bytes[0] === 0xfe && bytes[1] === 0xff) {
    encoding = bytes[0] === 0xff ? "utf-16le" : "utf-16be";
    value = new TextDecoder(encoding, { fatal: true }).decode(bytes);
  } else {
    try {
      value = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
      if (bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) encoding = "utf-8-bom";
    } catch {
      try { value = new TextDecoder("gb18030", { fatal: true }).decode(bytes); encoding = "gb18030"; }
      catch { value = new TextDecoder("utf-8").decode(bytes); encoding = "unknown"; }
    }
  }
  // Binary files should not masquerade as editable text.
  if (value.includes("\0")) throw new Error("binary");
  return { value, encoding, readOnly: encoding === "gb18030" || encoding === "unknown" };
}

export function encodeTextFile(value: string, encoding: TextEncoding): Uint8Array<ArrayBuffer> {
  if (encoding === "unknown" || encoding === "gb18030") throw new Error("read-only encoding");
  if (encoding === "utf-16le" || encoding === "utf-16be") {
    const bytes = new Uint8Array(2 + value.length * 2);
    const view = new DataView(bytes.buffer);
    const little = encoding === "utf-16le";
    view.setUint16(0, 0xfeff, little);
    for (let i = 0; i < value.length; i++) view.setUint16(2 + i * 2, value.charCodeAt(i), little);
    return bytes;
  }
  const bytes = new TextEncoder().encode(value);
  if (encoding !== "utf-8-bom") return bytes;
  const result = new Uint8Array(bytes.length + 3);
  result.set([0xef, 0xbb, 0xbf]); result.set(bytes, 3);
  return result;
}

export function textFileLanguage(name: string): string {
  const extension = name.toLowerCase().split(".").at(-1) ?? "";
  if (/^dockerfile$/i.test(name)) return "dockerfile";
  const languages: Record<string, string> = { json: "json", jsonc: "json", yml: "yaml", yaml: "yaml", md: "markdown", markdown: "markdown", js: "javascript", jsx: "javascript", ts: "typescript", tsx: "typescript", py: "python", rs: "rust", go: "go", sh: "shell", bash: "shell", ps1: "powershell", bat: "bat", cmd: "bat", html: "html", htm: "html", css: "css", scss: "scss", sql: "sql", xml: "xml", c: "c", h: "c", cc: "cpp", cpp: "cpp", hpp: "cpp", java: "java", php: "php", rb: "ruby", ini: "ini", toml: "ini", conf: "ini" };
  return languages[extension] ?? "plaintext";
}
