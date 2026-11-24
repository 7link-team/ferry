import { useEffect, useState } from "react";
import { MessagesSquare } from "lucide-react";
import { api } from "../../shared/api/api";
import { currentLocale, localizedPath } from "../../shared/i18n/locale";
import { conversationText as t } from "./resource-config";

/** "Created in conversation X" for a pipeline or workflow created from a room the viewer can see. */
export function ResourceOrigin({ workspace, kind, id, permissions }: {
  workspace: string; kind: "pipeline" | "workflow"; id: string | undefined; permissions: Set<string>;
}) {
  const [rooms, setRooms] = useState<{ conversation_id: string; title: string }[]>([]);
  const allowed = permissions.has("conversations.read");
  useEffect(() => {
    if (!allowed || !id) return;
    let live = true;
    void api<{ items: { conversation_id: string; title: string }[] }>(`/workspaces/${encodeURIComponent(workspace)}/resource-conversations/${kind}/${encodeURIComponent(id)}`)
      .then(result => { if (live) setRooms(result.items); })
      .catch(() => { if (live) setRooms([]); });
    return () => { live = false; };
  }, [workspace, kind, id, allowed]);
  if (!rooms.length) return null;
  return <p data-resource-origin className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-muted-foreground">
    <MessagesSquare className="size-4" aria-hidden />
    <span>{t("createdInConversation")}:</span>
    {rooms.map(room => <a key={room.conversation_id} className="text-primary underline [overflow-wrap:anywhere]"
      href={localizedPath(`/workspaces/${encodeURIComponent(workspace)}/conversations/${room.conversation_id}`, currentLocale())}>{room.title}</a>)}
  </p>;
}
