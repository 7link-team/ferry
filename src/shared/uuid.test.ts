import { afterEach, expect, it, vi } from "vitest";
import { randomUUID } from "./uuid";

afterEach(() => vi.unstubAllGlobals());

it("uses the native UUID generator when available", () => {
  const native = vi.fn(() => "12345678-1234-4234-8234-123456789abc");
  vi.stubGlobal("crypto", { randomUUID: native });
  expect(randomUUID()).toBe("12345678-1234-4234-8234-123456789abc");
  expect(native).toHaveBeenCalledOnce();
});

it("generates a standard UUID from secure random bytes on HTTP origins", () => {
  const bytes = vi.fn((value: Uint8Array) => {
    value.set(Array.from({ length: 16 }, (_, index) => index));
    return value;
  });
  vi.stubGlobal("crypto", { getRandomValues: bytes });
  expect(randomUUID()).toBe("00010203-0405-4607-8809-0a0b0c0d0e0f");
  expect(bytes).toHaveBeenCalledOnce();
});

it("does not replace a failed secure random source with weak randomness", () => {
  vi.stubGlobal("crypto", { getRandomValues: () => { throw Error("Random source unavailable"); } });
  expect(() => randomUUID()).toThrow("Random source unavailable");
});
