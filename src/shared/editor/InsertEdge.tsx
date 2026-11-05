import { BaseEdge, EdgeLabelRenderer, getBezierPath, type Edge, type EdgeProps } from '@xyflow/react';
import { Plus } from 'lucide-react';
import { workflowText } from '../i18n/i18n';
import { Button } from '../ui/button';

export type InsertableEdge = Edge<{ insert?: () => void }>;
/** The same accessible insertion affordance serves both graph editors. */
export function InsertEdge(props: EdgeProps<InsertableEdge>) {
  const [path, x, y] = getBezierPath(props);
  return <>
    <BaseEdge id={props.id} path={path} {...(props.markerEnd ? { markerEnd: props.markerEnd } : {})} {...(props.style ? { style: props.style } : {})} interactionWidth={24} />
    {props.data?.insert && <EdgeLabelRenderer>
      <Button type="button" variant="outline" size="icon-sm" className="nodrag nopan absolute rounded-full border-2 shadow-sm" style={{ pointerEvents: 'all', transform: `translate(-50%, -50%) translate(${x}px, ${y}px)` }} aria-label={workflowText('insert')} title={workflowText('insert')} onClick={event => { event.stopPropagation(); props.data?.insert?.(); }}><Plus className="size-4" /></Button>
    </EdgeLabelRenderer>}
  </>;
}
