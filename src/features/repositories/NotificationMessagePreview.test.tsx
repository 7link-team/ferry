import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import { NotificationMessagePreview } from "./NotificationMessagePreview";

it("renders the actual WeCom payload with inline labels, native colors and safe links", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div"), root = createRoot(host);
  const content = '### <font color="comment">代码推送</font>\n> 仓库：[7link-team/ferry](https://example.invalid/repo)\n> **分支**：main\n> <font color="warning">提交信息：feat: preview notifications</font>\n> <font color="info">新增：421 行</font>\n> <font color="warning">删除：91 行</font>';
  try {
    await act(async () => root.render(<NotificationMessagePreview provider="WeCom" payload={{markdown:{content}}} />));
    expect(host.querySelector('dl')).toBeNull();
    expect(host.querySelector('blockquote')?.textContent).toContain('分支：main');
    const commit = [...host.querySelectorAll('span')].find(span => span.textContent === '提交信息：feat: preview notifications')!;
    expect(commit.style.color).toBe('rgb(250, 157, 59)');
    expect((host.querySelector('h5 span') as HTMLElement).style.color).toBe('rgb(144, 147, 153)');
    expect(host.querySelector('a')?.href).toBe('https://example.invalid/repo');
    expect(host.querySelector('font')).toBeNull();
    expect(host.textContent).not.toContain('<font');
  } finally { await act(async () => root.unmount()); }
});

it("uses Feishu's header and entire field color, without imposing colors on generic JSON", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host=document.createElement('div'),root=createRoot(host);
  try {
    await act(async()=>root.render(<NotificationMessagePreview provider="Feishu" payload={{card:{header:{template:'blue',title:{content:'代码推送'}},elements:[{text:{content:"<font color='blue'>提交信息：feat: x</font>"}},{text:{content:"<font color='green'>新增：12 行</font>"}}]}}} />));
    expect(host.querySelector('h5')?.textContent).toBe('代码推送');
    const span=[...host.querySelectorAll('span')].find(span=>span.textContent==='提交信息：feat: x')!;
    expect(span.style.color).toBe('rgb(51, 112, 255)');
    await act(async()=>root.render(<NotificationMessagePreview provider="Webhook" payload={{test:true,message:'<unsafe>'}} />));
    expect(host.querySelector('pre')?.textContent).toContain('"message": "<unsafe>"');
    expect(host.querySelector('h5')).toBeNull();
  } finally {await act(async()=>root.unmount());}
});

it("keeps raw HTML inert and rejects unsafe links in native previews", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host=document.createElement('div'),root=createRoot(host);
  try {
    await act(async()=>root.render(<NotificationMessagePreview provider="WeCom" payload={{markdown:{content:'<script>alert(1)</script>\n\n[unsafe](javascript:alert) ![image](https://example.invalid/image) <font color="warning" onclick="alert(1)">text</font>'}}} />));
    expect(host.querySelector('script,img,font,[onclick]')).toBeNull();
    expect(host.querySelector('a[href^="javascript"]')).toBeNull();
    await act(async()=>root.render(<NotificationMessagePreview provider="Telegram" payload={{text:'Code push\nRepository：<a href="https://example.invalid/repo">example/app</a>\n&lt;sample&gt;'}} />));
    expect(host.textContent).toContain('Repository：example/app\n<sample>');
    expect(host.querySelector('a')?.href).toBe('https://example.invalid/repo');
  } finally {await act(async()=>root.unmount());}
});
