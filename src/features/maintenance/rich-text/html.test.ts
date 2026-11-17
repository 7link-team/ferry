import { describe, expect, it } from "vitest";
import { sanitizeRichText } from "../value-content";
import { canonicalHtml, htmlToValue, isPlaceholder, PLACEHOLDER, roundTrips, valueToHtml } from "./html";

const round = (html: string) => roundTrips(html, sanitizeRichText);

describe("stored rich text round-trips through the visual editor", () => {
  it.each([
    ["empty", ""],
    ["empty paragraph", "<p><br></p>"],
    ["footer with links", '<p>© 2026 Example · <a href="https://example.invalid/privacy" target="_blank">Privacy</a> · contact@example.invalid</p>'],
    ["mail template", '<h1>Welcome</h1><p style="text-align: center;">Hello <strong>{{ user.name }}</strong>, your code is <code>{{code}}</code>.</p><ul><li>First</li><li>Second<ul><li>Nested</li></ul></li></ul><ol><li><p>Numbered</p></li></ol><blockquote>Quote</blockquote>'],
    ["table", '<table><tbody><tr><th>Name</th><th colspan="2">Value</th></tr><tr><td>A</td><td><p>One</p></td><td>Two</td></tr></tbody></table>'],
    ["wangEditor table defaults", '<table style="width: auto;"><tbody><tr><th colSpan="1" rowSpan="1" width="auto">H</th></tr><tr><td colSpan="1" rowSpan="1" width="auto">C</td></tr></tbody></table>'],
    ["code block", '<pre><code class="language-js">const a = 1;\n  if (a) {}</code></pre>'],
    ["image", '<p><img src="https://example.invalid/logo.png" alt="Logo"></p><p>Text <em>with <u>marks</u></em> and <s>strike</s>.<br>Second line</p>'],
    ["notification body", "<p>{{title}}</p><p>Built {{ run.number }} for {{repo}} — <b>{{status}}</b></p>"],
  ])("%s keeps its content and formatting", (_, html) => {
    expect(round(html)).toBe(true);
  });

  it.each([
    ["inline colour", '<p><span style="color: rgb(255, 0, 0);">Red</span></p>'],
    ["font size", '<p style="font-size: 18px;">Big</p>'],
    ["image size", '<img src="https://example.invalid/a.png" width="120">'],
    ["unsupported tag", "<p>Text</p><hr><p>After</p>"],
    ["heading level four", "<h4>Small heading</h4>"],
    ["block inside a quote", "<blockquote><p>One</p><p>Two</p></blockquote>"],
  ])("%s is kept for source editing instead of being dropped", (_, html) => {
    expect(round(html)).toBe(false);
  });

  it("writes plain, editor-free HTML in a stable form", () => {
    const value = htmlToValue('<p style="text-align: right;">Hi <b>{{name}}</b></p><ul><li>One</li></ul><table><tr><td>Cell</td></tr></table>')!;
    expect(valueToHtml(value)).toBe('<p style="text-align: right;">Hi <strong>{{name}}</strong></p><ul><li>One</li></ul><table><tbody><tr><td>Cell</td></tr></tbody></table>');
    expect(valueToHtml(htmlToValue("")!)).toBe("");
    expect(canonicalHtml("<p><br></p>")).toBe(canonicalHtml(""));
  });

  it("turns template variables into chips that serialize to exactly the same text", () => {
    const value = htmlToValue("<p>Dear {{ user.name }}, run {{run.id}} {{not a var}}</p>")!;
    const chips = JSON.stringify(value).match(new RegExp(`"type":"${PLACEHOLDER}","name":"([^"]+)"`, "g"));
    expect(chips).toHaveLength(2);
    expect(valueToHtml(value)).toBe("<p>Dear {{ user.name }}, run {{run.id}} {{not a var}}</p>");
    expect(isPlaceholder("{{ a.b }}")).toBe(true);
    expect(isPlaceholder("x {{a}}")).toBe(false);
  });

  it("escapes text and attributes and drops scripts through the sanitizer", () => {
    const value = htmlToValue('<p>a &lt;b&gt; &amp; "q" <a href="https://e.invalid/?a=1&amp;b=&quot;2&quot;">link</a></p>')!;
    const html = valueToHtml(value);
    expect(html).toBe('<p>a &lt;b&gt; &amp; "q" <a href="https://e.invalid/?a=1&amp;b=&quot;2&quot;">link</a></p>');
    expect(round('<p onclick="x()">Hi</p><script>alert(1)</script>')).toBe(true);
  });
});
