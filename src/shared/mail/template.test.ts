import { expect, it } from "vitest";
import { mailPreview, plainMailHtml } from "./template";

it("shows a styled default and resolves variables in the isolated HTML preview", () => {
  const html = mailPreview("Verify your email", plainMailHtml("Your code: {{code}}\nValid {{expires_minutes}} minutes."));
  expect(html).toContain('max-width:600px');
  expect(html).toContain('font-size:28px');
  expect(html).toContain("123456");
  expect(html).toContain("Valid 10 minutes.");
  expect(html).not.toContain("{{content}}");
});
it("escapes subjects and old plain bodies and removes active preview content", () => {
  const html = mailPreview('<img src=x onerror=alert(1)>', '<p>Hello {{email}}</p><script>alert(1)</script><img src=x onerror="alert(2)">');
  expect(html).toContain("&lt;img");
  expect(html).not.toContain("<script");
  expect(html).not.toContain('onerror="');
  expect(html).toContain("user@example.com");
  expect(plainMailHtml("<b>old literal text</b>")).toContain("&lt;b&gt;");
});
