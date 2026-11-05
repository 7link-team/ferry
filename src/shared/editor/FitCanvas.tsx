import { useNodesInitialized, useReactFlow } from '@xyflow/react';
import { useEffect, useRef } from 'react';

/** Fit after topology/layout changes and container resizing, never while dragging. */
export function FitCanvas({ topology }: { topology: string }) {
  const { fitView } = useReactFlow();
  const initialized = useNodesInitialized();
  const marker = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (!initialized) return;
    let timer: ReturnType<typeof setTimeout>;
    const fit = () => { clearTimeout(timer); timer = setTimeout(() => void fitView({ padding: 0.18, maxZoom: 1 }), 80); };
    const observer = new ResizeObserver(fit);
    if (marker.current?.parentElement) observer.observe(marker.current.parentElement);
    fit();
    return () => { clearTimeout(timer); observer.disconnect(); };
  }, [initialized, topology, fitView]);
  return <span ref={marker} hidden />;
}
