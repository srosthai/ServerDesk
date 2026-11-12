import type { ReactNode } from "react";

function SettingsSection({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <section className="grid gap-4 border-t border-border pt-6 md:grid-cols-[200px_minmax(0,1fr)] md:gap-10">
      <div>
        <h2 className="text-[14px] font-semibold tracking-[-0.01em]">{title}</h2>
        <p className="mt-1 text-[12px] leading-relaxed text-muted-foreground">{description}</p>
      </div>
      <div className="min-w-0">{children}</div>
    </section>
  );
}

export { SettingsSection };
