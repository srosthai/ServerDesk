import { Link } from "react-router-dom";
import { TerminalWindowIcon } from "@phosphor-icons/react";
import { FeaturePlaceholder } from "@/components/shared/FeaturePlaceholder";
import { Button } from "@/components/ui/button";

function TerminalPage() {
  return (
    <FeaturePlaceholder
      title="Terminal"
      description="A full interactive shell over the selected SSH session."
      icon={<TerminalWindowIcon className="size-4" aria-hidden="true" />}
      emptyTitle="No server selected"
      emptyDescription="Choose a server to open an interactive terminal with ANSI output and resize support."
      action={
        <Button asChild variant="secondary" size="sm">
          <Link to="/servers">Open servers</Link>
        </Button>
      }
    />
  );
}

export { TerminalPage };
