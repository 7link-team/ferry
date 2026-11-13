import { useMemo,useState,type ReactNode } from "react";
import { toast } from "sonner";
import type { Identity } from "../shared/api/api";
import { commonText,useCommonTranslation } from "../shared/i18n/i18n";
import type { FormConfig } from "../shared/resources/types";
import { Button } from "../shared/ui/button";
import { ResourceForm } from "./resources/ResourceForm";
import { navigate } from "./router";
import { AccountOtp } from "./AccountOtp";
import { AvatarEditor, type AvatarValue } from "../shared/ui/identity-avatar";
import { AppHeader } from "./AppHeader";
import { ThemeSwitcher } from "../shared/theme/ThemeSwitcher";
import { LanguageSwitcher } from "../shared/i18n/LanguageSwitcher";

type Row = Record<string, unknown> & { id: string };

/** The signed-in user's own account: one record at /api/profile/1. */
const profile = (userId: string): FormConfig<Row> => ({
  resource: "profile", endpoint: "/profile", heading: () => commonText("account.title"), blank: {}, sections: [
    { group: "basics", groupKey: "account.basics", title: "基本信息", titleKey: "common.basic", fields: [
      { key: "avatar", label: "Avatar", labelKey: "identity:avatar", type: "custom", span: 2,
        render: ({ draft, readOnly, onPatch, onValidityChange }) => <AvatarEditor id={userId} name={String(draft.name ?? "")} kind="user" value={draft.avatar as AvatarValue | undefined} readOnly={readOnly} onBusyChange={onValidityChange} onChange={avatar => onPatch({ avatar })} /> },
      { key: "name", label: "姓名", labelKey: "auth.name", type: "text", required: true },
    ] },
    { group: "basics", groupKey: "account.basics", title: "偏好设置", titleKey: "account.preferences", description: "主题和语言立即生效并自动保存；下方保存、取消按钮仅处理资料修改。", descriptionKey: "account.preferencesHelp", fields: [
      { key: "theme_preferences", label: "主题", labelKey: "theme:label", type: "custom", render: () => <ThemeSwitcher showLabel /> },
      { key: "language_preferences", label: "语言", labelKey: "language.label", type: "custom", render: () => <LanguageSwitcher showLabel /> },
    ] },
    { group: "security", groupKey: "account.security", title: "登录邮箱", titleKey: "account.loginEmail", fields: [
      { key: "email", label: "登录邮箱", labelKey: "account.loginEmail", type: "text", required: true, help: "修改后用新邮箱登录，其他设备上的登录会失效。", helpKey: "account.emailHelp" },
    ] },
    { group: "security", groupKey: "account.security", title: "修改密码", titleKey: "account.changePassword", fields: [
      { key: "new_password", label: "新密码", labelKey: "account.newPassword", type: "password", help: "留空则不修改；至少 8 个字符。修改后其他设备上的登录会失效。", helpKey: "account.passwordHelp" },
    ] },
    { group: "security", groupKey: "account.security", title: "二步验证", titleKey: "otp:title", fields: [
      { key: "otp", label: "二步验证", labelKey: "otp:title", type: "custom", span: 2, render: () => <AccountOtp /> },
    ] },
    { group: "security", groupKey: "account.security", title: "身份确认", titleKey: "account.verify", fields: [
      { key: "current_password", label: "当前密码", labelKey: "account.currentPassword", type: "password", help: "修改登录邮箱或密码时必须填写；只改姓名可留空。", helpKey: "account.verifyHelp" },
    ] },
  ],
});
const permissions = new Set(["profile.update"]);
export const accountRoute = { workspace: "_account", group: "account", item: "profile", id: "1", mode: "edit" } as const;

export function Account({ identity, reload, headerActions }: { identity: Identity; reload: () => void; headerActions?: ReactNode }) {
  useCommonTranslation();
  const config = useMemo(() => profile(identity.user.id), [identity.user.id]);
  // /account always parses to the edit view; remount after saving or cancelling
  // so the form reloads the stored record and clears the password inputs.
  const [saves, setSaves] = useState(0);
  const back = () => navigate(identity.user.is_superadmin && !identity.workspaces.length
    ? { workspace: "_platform", group: "platform", item: "workspaces", mode: "list" }
    : { workspace: identity.workspaces[0]?.id ?? "_platform", group: "dashboard", item: "overview", mode: "list" }, {});
  return <div className="h-full min-w-0 overflow-auto bg-background">
    <AppHeader navigation={<Button variant="ghost" className="min-h-11 shrink-0" onClick={back}>{commonText("account.back")}</Button>} actions={headerActions} />
    <main className="mx-auto w-full max-w-5xl p-4 sm:p-6">
      <h1 className="mb-6 text-xl font-semibold">{commonText("account.title")}</h1>
      <ResourceForm key={saves} config={config} workspace="_account" permissions={permissions} showHeading={false}
        onCancel={() => setSaves(count => count + 1)}
        onSaved={() => { toast.success(commonText("account.saved")); setSaves(count => count + 1); reload(); }} />
    </main>
  </div>;
}
