import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { PlatformFrame } from "./PlatformFooter";
import * as api from "../shared/api/api";

let host: HTMLDivElement, root: Root;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.useFakeTimers();
  vi.spyOn(api, "api").mockResolvedValue({content:""});
  host=document.createElement("div");document.body.append(host);root=createRoot(host);
});
afterEach(async () => {
  await act(async()=>root.unmount());host.remove();
  vi.useRealTimers();vi.restoreAllMocks();vi.unstubAllGlobals();
});
it("tracks visual viewport and focus without remounting the editor, preserving pinch zoom", async () => {
  const viewport=Object.assign(new EventTarget(), {height:667,scale:1,offsetTop:0});
  vi.stubGlobal("visualViewport",viewport);
  await act(async()=>root.render(<PlatformFrame><textarea defaultValue="Unsent draft" /></PlatformFrame>));
  const input=host.querySelector('textarea')!;
  const html=document.documentElement;
  expect(html.style.getPropertyValue('--app-viewport-height')).toBe('667px');
  await act(async()=>{input.focus();viewport.height=360;viewport.offsetTop=25;viewport.dispatchEvent(new Event('resize'));await vi.advanceTimersByTimeAsync(20);});
  expect(html.hasAttribute('data-app-input-active')).toBe(true);
  expect(html.style.getPropertyValue('--app-viewport-height')).toBe('360px');
  expect(html.style.getPropertyValue('--app-viewport-top')).toBe('25px');
  expect(host.querySelector('textarea')).toBe(input);expect(input.value).toBe('Unsent draft');
  await act(async()=>{viewport.scale=2;viewport.height=180;viewport.dispatchEvent(new Event('resize'));await vi.advanceTimersByTimeAsync(20);});
  expect(html.style.getPropertyValue('--app-viewport-height')).toBe('360px');
  await act(async()=>{input.blur();await vi.advanceTimersByTimeAsync(20);});
  expect(html.hasAttribute('data-app-input-active')).toBe(false);
  await act(async()=>root.render(<div/>));
  expect(html.style.getPropertyValue('--app-viewport-height')).toBe('');
  expect(html.style.getPropertyValue('--app-viewport-top')).toBe('');
});
it("uses the layout viewport when visualViewport is unavailable and ignores readonly focus", async () => {
  vi.stubGlobal('visualViewport',undefined);vi.stubGlobal('innerHeight',360);
  await act(async()=>root.render(<PlatformFrame><input readOnly value="Read only"/></PlatformFrame>));
  await act(async()=>{host.querySelector('input')!.focus();await vi.advanceTimersByTimeAsync(20);});
  expect(document.documentElement.style.getPropertyValue('--app-viewport-height')).toBe('360px');
  expect(document.documentElement.hasAttribute('data-app-input-active')).toBe(false);
});

it("remeasures a restored page without requiring a resize or remounting its draft", async () => {
  const viewport=Object.assign(new EventTarget(),{height:667,scale:1,offsetTop:0});
  vi.stubGlobal('visualViewport',viewport);
  await act(async()=>root.render(<PlatformFrame><textarea defaultValue="Keep draft"/></PlatformFrame>));
  const input=host.querySelector('textarea');
  viewport.height=844;viewport.offsetTop=0;
  await act(async()=>{window.dispatchEvent(new Event('pageshow'));await vi.advanceTimersByTimeAsync(20);});
  expect(document.documentElement.style.getPropertyValue('--app-viewport-height')).toBe('844px');
  expect(host.querySelector('textarea')).toBe(input);
  expect(input?.value).toBe('Keep draft');
});
