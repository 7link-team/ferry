import { useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import type { Identity } from "./api";
import { ResourceForm } from "./components/ResourceForm";
import type { FormConfig } from "./components/dsl";
import { navigate } from "./router";

type Row = Record<string, unknown> & { id: string };

/** The signed-in user's own account: one record at /api/profile/1. */
const profile: FormConfig<Row> = {
  resource: "profile", endpoint: "/profile", heading: () => "个人资料", blank: {}, sections: [
    { title: "基本信息", fields: [
      { key: "name", label: "姓名", type: "text", required: true },
      { key: "email", label: "登录邮箱", type: "text", required: true, help: "修改后用新邮箱登录，其他设备上的登录会失效。" },
    ] },
    { title: "修改密码", fields: [
      { key: "new_password", label: "新密码", type: "password", help: "留空则不修改；至少 8 个字符。修改后其他设备上的登录会失效。" },
    ] },
    { title: "身份确认", fields: [
      { key: "current_password", label: "当前密码", type: "password", help: "修改登录邮箱或密码时必须填写；只改姓名可留空。" },
    ] },
  ],
};
const permissions = new Set(["profile.update"]);
export const accountRoute = { workspace: "_account", group: "account", item: "profile", id: "1", mode: "edit" } as const;

export function Account({ identity, reload }: { identity: Identity; reload: () => void }) {
  // /account always parses to the edit view; remount after saving or cancelling
  // so the form reloads the stored record and clears the password inputs.
  const [saves, setSaves] = useState(0);
  const back = () => navigate(identity.user.is_superadmin && !identity.workspaces.length
    ? { workspace: "_platform", group: "platform", item: "users", mode: "list" }
    : { workspace: identity.workspaces[0]?.id ?? "_platform", mode: "list" }, {});
  return <div className="flex h-full flex-col">
    <header className="flex flex-wrap items-center gap-4 border-b px-5 py-3">
      <h1 className="font-semibold">个人资料</h1>
      <span className="flex-1" /><span className="text-sm text-muted-foreground">{identity.user.email}</span>
      <Button variant="ghost" onClick={back}>返回</Button>
    </header>
    <main className="min-h-0 flex-1 overflow-auto p-5">
      <ResourceForm key={saves} config={profile} workspace="_account" permissions={permissions}
        onCancel={() => setSaves(count => count + 1)}
        onSaved={() => { toast.success("个人资料已保存。"); setSaves(count => count + 1); reload(); }} />
    </main>
  </div>;
}
