import type * as React from "react";
import { EmptyState } from "@/components/ui/empty-state";
import { PageHeader } from "@/components/ui/page-header";

type FeaturePlaceholderProps = {
  title: string;
  description: string;
  icon: React.ReactNode;
  emptyTitle: string;
  emptyDescription: string;
  action?: React.ReactNode;
};

function FeaturePlaceholder({
  title,
  description,
  icon,
  emptyTitle,
  emptyDescription,
  action,
}: FeaturePlaceholderProps) {
  return (
    <div className="space-y-6">
      <PageHeader title={title} description={description} />
      <EmptyState icon={icon} title={emptyTitle} description={emptyDescription} action={action} />
    </div>
  );
}

export { FeaturePlaceholder };
