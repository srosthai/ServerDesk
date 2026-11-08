import { SlidersHorizontalIcon } from "@phosphor-icons/react";
import { FeaturePlaceholder } from "@/components/shared/FeaturePlaceholder";

function SettingsPage() {
  return (
    <FeaturePlaceholder
      title="Settings"
      description="Appearance, monitoring cadence, and local application preferences."
      icon={<SlidersHorizontalIcon className="size-4" aria-hidden="true" />}
      emptyTitle="No preferences yet"
      emptyDescription="Theme, polling interval, and other local preferences will appear here."
    />
  );
}

export { SettingsPage };
