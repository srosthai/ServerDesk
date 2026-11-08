import { ShieldCheckIcon } from "@phosphor-icons/react";
import { FeaturePlaceholder } from "@/components/shared/FeaturePlaceholder";

function SecurityPage() {
  return (
    <FeaturePlaceholder
      title="Security & Credentials"
      description="Local credential storage and trusted host keys."
      icon={<ShieldCheckIcon className="size-4" aria-hidden="true" />}
      emptyTitle="Secrets stay on this device"
      emptyDescription="SSH passwords and key passphrases will be stored in the operating system credential store, never in SQLite or logs."
    />
  );
}

export { SecurityPage };
