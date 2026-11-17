import { describe, expect, it } from "vitest";
import { sanitizeRichText } from "../value-content";
import { plainMailHtml } from "../../../shared/mail/template";
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
    ["wangEditor coloured text", '<p>Status: <span style="color: rgb(225, 60, 57);">failed</span> and <span style="color: rgb(19, 194, 194);"><strong>{{ run.id }}</strong></span></p>'],
    ["wangEditor background colour", '<p><span style="background-color: rgb(255, 236, 61);">Highlighted</span> text</p>'],
    ["wangEditor font size", '<p><span style="font-size: 14px;">Small</span> and <span style="font-size: 24px;"><em>large</em></span></p>'],
    ["combined styles in either nesting order", '<p><span style="color: rgb(0, 0, 255);"><span style="background-color: rgb(255, 255, 0);">Both</span></span> <span style="background-color: #ff0; color: #00f; font-size: 16px;">all</span></p>'],
    ["horizontal rule", "<p>Before</p><hr/><p>After</p>"],
    ["headings four to six", "<h4>Four</h4><h5>Five</h5><h6 style=\"text-align: center;\">Six</h6>"],
    ["wangEditor image width", '<p><img src="https://example.invalid/a.png" alt="" style="width: 300px;"></p>'],
    ["image size attributes", '<img src="https://example.invalid/a.png" width="120" height="40">'],
    ["image width and height styles", '<p><img src="https://example.invalid/a.png" alt="A" style="width: 50%; height: 80px;"/></p>'],
  ])("%s keeps its content and formatting", (_, html) => {
    expect(round(html)).toBe(true);
  });

  it.each([
    ["block font size", '<p style="font-size: 18px;">Big</p>'],
    ["font family", '<p><span style="font-family: Georgia;">Serif</span></p>'],
    ["line height", '<p style="line-height: 2;">Tall</p>'],
    ["image float", '<img src="https://example.invalid/a.png" style="float: left;">'],
    ["unsupported tag", "<p>Text</p><details><summary>More</summary>Hidden</details>"],
    ["block inside a quote", "<blockquote><p>One</p><p>Two</p></blockquote>"],
  ])("%s is kept for source editing instead of being dropped", (_, html) => {
    expect(round(html)).toBe(false);
  });

  it("keeps colour, size, rules and image sizes as editor values", () => {
    const value = htmlToValue('<p><span style="color: rgb(255, 0, 0); font-size: 14px;">Red</span></p><hr><h5>H</h5><p><img src="https://e.invalid/a.png" style="width: 300px;"></p>')!;
    expect(value[0]!.children[0]).toEqual({ text: "Red", color: "rgb(255, 0, 0)", fontSize: "14px" });
    expect(value[1]!.type).toBe("hr");
    expect(value[2]!.type).toBe("h5");
    expect(value[3]).toMatchObject({ type: "img", cssWidth: "300px" });
    expect(valueToHtml(value)).toBe('<p><span style="color: rgb(255, 0, 0); font-size: 14px;">Red</span></p><hr><h5>H</h5><img src="https://e.invalid/a.png" style="width: 300px;">');
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

it("opens the built-in verification mail template in the visual editor", () => {
  // The default body must stay editable visually (not fall back to HTML source).
  expect(round(plainMailHtml(["您的验证码是 {{code}}", "{{expires_minutes}} 分钟内有效。"].join("\n")))).toBe(true);
});
