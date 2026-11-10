/** Shared brand mark. The adjacent site name provides its accessible label. */
export function BrandIcon({ className = "size-8" }: { className?: string }) {
  return <img src="/brand/logo-256.png" srcSet="/brand/logo-256.png 1x, /brand/logo-512.png 2x" alt="" aria-hidden="true" width="256" height="256" className={`shrink-0 object-contain ${className}`} draggable={false} />;
}
