import { ClockCounterClockwiseIcon } from "@phosphor-icons/react";
import { FeaturePlaceholder } from "@/components/shared/FeaturePlaceholder";

function HistoryPage() {
  return (
    <FeaturePlaceholder
      title="Command History"
      description="An audit trail of actions performed in ServerDesk."
      icon={<ClockCounterClockwiseIcon className="size-4" aria-hidden="true" />}
      emptyTitle="No operations recorded"
      emptyDescription="Completed actions will appear here with the server, result, time, and duration. Secrets are never recorded."
    />
  );
}

export { HistoryPage };
