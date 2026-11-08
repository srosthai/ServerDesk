import { HardDrivesIcon } from "@phosphor-icons/react";
import { FeaturePlaceholder } from "@/components/shared/FeaturePlaceholder";

function ServersPage() {
  return (
    <FeaturePlaceholder
      title="Servers"
      description="Saved VPS profiles, connection details, and grouping."
      icon={<HardDrivesIcon className="size-4" aria-hidden="true" />}
      emptyTitle="No server profiles yet"
      emptyDescription="Server profiles are not available in this build. The next milestone adds connection details, credentials, and host verification."
    />
  );
}

export { ServersPage };
