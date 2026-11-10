import { useState,type ReactNode } from "react";
import { toast } from "sonner";
import type { Identity } from "../shared/api/api";
import { commonText,useCommonTranslation } from "../shared/i18n/i18n";
import type { FormConfig } from "../shared/resources/types";
import { Button } from "../shared/ui/button";
import { ResourceForm } from "./resources/ResourceForm";
import { navigate } from "./router";
import { AccountOtp } from "./AccountOtp";

type Row = Record<string, unknown> & { id: string };

/** The signed-in user's own account: one record at /api/profile/1. */
const profile: FormConfig<Row> = {
  resource: "profile", endpoint: "/profile", heading: () => commonText("account.title"), blank: {}, sections: [
    { title: "基本信息", titleKey: "common.basic", fields: [
      { key: "name", label: "姓名", labelKey: "auth.name", type: "text", required: true },
      { key: "email", label: "登录邮箱", labelKey: "account.loginEmail", type: "text", required: true, help: "修改后用新邮箱登录，其他设备上的登录会失效。", helpKey: "account.emailHelp" },
    ] },
    { title: "修改密码", titleKey: "account.changePassword", fields: [
      { key: "new_password", label: "新密码", labelKey: "account.newPassword", type: "password", help: "留空则不修改；至少 8 个字符。修改后其他设备上的登录会失效。", helpKey: "account.passwordHelp" },
    ] },
    { title: "二步验证", titleKey: "otp:title", fields: [
      { key: "otp", label: "二步验证", labelKey: "otp:title", type: "custom", span: 2, render: () => <AccountOtp /> },
    ] },
    { title: "身份确认", titleKey: "account.verify", fields: [
      { key: "current_password", label: "当前密码", labelKey: "account.currentPassword", type: "password", help: "修改登录邮箱或密码时必须填写；只改姓名可留空。", helpKey: "account.verifyHelp" },
    ] },
  ],
};
const permissions = new Set(["profile.update"]);
export const accountRoute = { workspace: "_account", group: "account", item: "profile", id: "1", mode: "edit" } as const;

export function Account({ identity, reload, headerActions }: { identity: Identity; reload: () => void; headerActions?: ReactNode }) {
  useCommonTranslation();
  // /account always parses to the edit view; remount after saving or cancelling
  // so the form reloads the stored record and clears the password inputs.
  const [saves, setSaves] = useState(0);
  const back = () => navigate(identity.user.is_superadmin && !identity.workspaces.length
    ? { workspace: "_platform", group: "platform", item: "workspaces", mode: "list" }
    : { workspace: identity.workspaces[0]?.id ?? "_platform", mode: "list" }, {});
  return <div className="flex h-full flex-col">
    <header className="flex flex-wrap items-center gap-4 border-b px-5 py-3">
      <h1 className="font-semibold">{commonText("account.title")}</h1>
      <span className="flex-1" /><span className="text-sm text-muted-foreground">{identity.user.email}</span>
      <Button variant="ghost" onClick={back}>{commonText("account.back")}</Button>
      {headerActions}
    </header>
    <main className="min-h-0 flex-1 overflow-auto p-5">
      <ResourceForm key={saves} config={profile} workspace="_account" permissions={permissions}
        onCancel={() => setSaves(count => count + 1)}
        onSaved={() => { toast.success(commonText("account.saved")); setSaves(count => count + 1); reload(); }} />
    </main>
  </div>;
}
