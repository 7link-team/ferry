import layout from "./layout.html?raw";
import { sanitizeRichText } from "../editor/sanitize-rich-text";

export const escapeHtml = (value: string) => value.replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"})[c]!);
export const plainMailHtml = (value: string) => value.split(/\r?\n/).map(line => `<p>${escapeHtml(line)}</p>`).join("").replace("{{code}}", '<strong style="font-size:28px;letter-spacing:6px;color:#2563eb">{{code}}</strong>');
export function mailPreview(subject: string, body: string, minutes = 10) {
  const render = (value: string) => value.replace(/\{\{(code|email|expires_minutes)\}\}/g, (_all, key: string) => ({code:"123456",email:"user@example.com",expires_minutes:String(minutes)})[key]!);
  return layout.replace("{{title}}", escapeHtml(render(subject))).replace("{{content}}", sanitizeRichText(render(body)));
}
