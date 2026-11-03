import { expect, it } from "vitest";
import { decodeTextFile, encodeTextFile, textFileLanguage } from "./text-file";

it.each(["utf-8", "utf-8-bom", "utf-16le", "utf-16be"] as const)("preserves %s bytes, CRLF and supplementary characters on save", encoding => {
  const value = "日志 🚢\r\nline 2\r\n";
  const bytes = encodeTextFile(value, encoding);
  const decoded = decodeTextFile(bytes);
  expect(decoded).toEqual({ value, encoding, readOnly: false });
  expect(encodeTextFile(decoded.value, decoded.encoding)).toEqual(bytes);
});
it("previews Windows Chinese logs without allowing destructive UTF-8 conversion", () => {
  const decoded = decodeTextFile(new Uint8Array([0xd6, 0xd0, 0xce, 0xc4, 13, 10]));
  expect(decoded).toEqual({ value: "中文\r\n", encoding: "gb18030", readOnly: true });
  expect(() => encodeTextFile(decoded.value, decoded.encoding)).toThrow();
});
it("shows malformed logs read-only and rejects binary data", () => {
  const decoded = decodeTextFile(new Uint8Array([0xff, 0xff, 0x0a]));
  expect(decoded.readOnly).toBe(true);
  expect(() => encodeTextFile(decoded.value, decoded.encoding)).toThrow();
  expect(() => decodeTextFile(new Uint8Array([65, 0, 66]))).toThrow("binary");
});
it.each([["test.json", "json"], ["test.yml", "yaml"], ["README.md", "markdown"], ["server.log", "plaintext"], ["Dockerfile", "dockerfile"]])("opens %s with %s syntax", (name, language) => {
  expect(textFileLanguage(name!)).toBe(language);
});
