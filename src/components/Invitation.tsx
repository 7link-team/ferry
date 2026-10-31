import { AutoField } from "./auto-form";
import { useState } from "react";
import { api, errorText } from "../api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

export function Invitation({ workspace }: { workspace: string }) {
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const load = async (rotate: boolean) => {
    if (rotate && !window.confirm("重新生成邀请码后，旧邀请码立即失效。继续？"))
      return;
    setBusy(true);
    setMessage("");
    try {
      const value = await api<{ code: string }>(
        `/workspaces/${workspace}/members/invitation`,
        { method: rotate ? "POST" : "GET", cache: "no-store" },
      );
      setCode(value.code);
      if (rotate) setMessage("已生成新邀请码，旧邀请码已失效。");
    } catch (cause) {
      setMessage(errorText(cause));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section
      className="mb-3 space-y-2 rounded-md border p-3"
      aria-label="邀请成员"
    >
      <div className="flex flex-wrap items-end gap-2">
        <Button
          variant="outline"
          disabled={busy}
          onClick={() => void load(false)}
        >
          查看邀请码
        </Button>
        {code && (
          <>
            <AutoField label="邀请码" className="text-sm">
              <Input readOnly value={code} />
            </AutoField>
            <Button
              variant="outline"
              onClick={async () => {
                try {
                  await navigator.clipboard.writeText(code);
                  setMessage("邀请码已复制。");
                } catch {
                  setMessage("复制失败，请手动复制邀请码。");
                }
              }}
            >
              复制邀请码
            </Button>
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => void load(true)}
            >
              重新生成邀请码
            </Button>
          </>
        )}
      </div>
      <p className="text-xs text-muted-foreground">
        对方登录后，在工作空间菜单选择“创建/加入工作空间”，填写邀请码即可加入，初始角色为只读。
      </p>
      {message && (
        <p role="status" className="text-sm">
          {message}
        </p>
      )}
    </section>
  );
}
