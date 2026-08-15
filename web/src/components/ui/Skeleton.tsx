interface SkeletonProps {
  className?: string;
  style?: React.CSSProperties;
}

export function Skeleton({ className = "", style }: SkeletonProps) {
  return (
    <div
      className={`skeleton-shimmer rounded ${className}`}
      style={style}
      aria-hidden
    />
  );
}

export function SettingsSkeleton() {
  return (
    <div className="space-y-6" aria-busy="true" aria-label="Loading settings">
      {/* Section title */}
      <Skeleton className="h-3.5 w-24" />

      {/* Form fields: label + input */}
      <div className="space-y-4">
        <div className="space-y-1.5">
          <Skeleton className="h-2.5 w-16" />
          <Skeleton className="h-8 w-full" />
        </div>
        <div className="space-y-1.5">
          <Skeleton className="h-2.5 w-20" />
          <Skeleton className="h-8 w-full" />
        </div>
        <div className="space-y-1.5">
          <Skeleton className="h-2.5 w-12" />
          <Skeleton className="h-24 w-full" />
        </div>
      </div>

      {/* Save button */}
      <Skeleton className="h-7 w-16" />
    </div>
  );
}

export function EnginesSkeleton() {
  return (
    <div className="space-y-6" aria-busy="true" aria-label="Loading engines">
      {/* Retrieval section */}
      <section className="space-y-3">
        <div className="flex items-center justify-between gap-2">
          <Skeleton className="h-3.5 w-32" />
          <Skeleton className="h-5 w-14 rounded-full" />
        </div>
        <div className="space-y-3 pl-4">
          <Skeleton className="h-3 w-48" />
          <div className="flex items-center gap-2">
            <Skeleton className="h-2.5 w-full" />
            <Skeleton className="h-6 w-6 shrink-0 rounded" />
            <Skeleton className="h-4 w-4 shrink-0 rounded" />
          </div>
        </div>
      </section>

      {/* LSP section */}
      <section className="space-y-3">
        <div className="flex items-center justify-between gap-2">
          <Skeleton className="h-3.5 w-28" />
          <Skeleton className="h-5 w-14 rounded-full" />
        </div>
        <div className="lsp-grid">
          {Array.from({ length: 4 }).map((_, i) => (
            <div key={i} className="tool-binding-card flex flex-col overflow-hidden">
              <div className="flex min-h-[60px] w-full items-start justify-between gap-2 p-3">
                <div className="min-w-0 space-y-1.5">
                  <Skeleton className="h-3.5 w-24" />
                  <Skeleton className="h-2.5 w-32" />
                </div>
                <Skeleton className="h-4 w-8 rounded-full" />
              </div>
              <div className="flex items-center justify-between gap-2 px-3 py-2.5">
                <Skeleton className="h-3 w-16" />
                <Skeleton className="h-6 w-16 rounded" />
              </div>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}

export function FileTreeSkeleton() {
  return (
    <div className="space-y-2 px-3 py-2" aria-busy="true" aria-label="Loading files">
      <Skeleton className="h-3.5 w-[55%]" />
      <Skeleton className="h-3.5 w-[72%]" />
      <Skeleton className="h-3.5 w-[48%]" />
      <Skeleton className="h-3.5 w-[65%]" />
      <Skeleton className="h-3.5 w-[58%]" />
    </div>
  );
}

export function MessageHistorySkeleton() {
  return (
    <div
      className="border-b border-(--_dk-line) px-4 py-3"
      aria-busy="true"
      aria-label="Loading earlier items"
    >
      <Skeleton className="mx-auto h-3 w-28" />
    </div>
  );
}
