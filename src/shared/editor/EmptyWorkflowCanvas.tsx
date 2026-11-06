import { Plus, Workflow } from "lucide-react";
import { Button } from "../ui/button";

/** Shared start state for both workflow editors. */
export function EmptyWorkflowCanvas({ title, description, action, disabled, onAdd }: {
  title: string; description: string; action: string; disabled?: boolean; onAdd: () => void;
}) {
  return <div data-empty-workflow-canvas className="pointer-events-none absolute inset-0 z-10 grid place-items-center px-6">
    <div className="max-w-sm text-center">
      <Workflow className="mx-auto mb-4 size-10 text-primary" aria-hidden="true" />
      <h3 className="font-medium">{title}</h3>
      <p className="mt-2 text-sm leading-6 text-muted-foreground">{description}</p>
      <Button type="button" className="pointer-events-auto mt-5" disabled={disabled} onClick={onAdd}><Plus />{action}</Button>
    </div>
  </div>;
}
