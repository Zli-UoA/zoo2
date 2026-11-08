import { Switch } from "@base-ui/react/switch";
import { useSyncExternalStore } from "react";

import {
  isCallSoundsEnabled,
  playCallSound,
  setCallSoundsEnabled,
  subscribeCallSounds,
} from "../../lib/call/call-sounds";

export function CallSoundsSwitch({ label }: { label: string }) {
  const enabled = useSyncExternalStore(subscribeCallSounds, isCallSoundsEnabled, () => true);
  return (
    <Switch.Root
      checked={enabled}
      aria-label={label}
      onCheckedChange={(checked) => {
        setCallSoundsEnabled(checked);
        // オンにした時に試聴できるようにする
        if (checked) playCallSound("join");
      }}
      className="relative inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full bg-input outline-none transition-colors focus-visible:ring-3 focus-visible:ring-ring/50 data-[checked]:bg-primary"
    >
      <Switch.Thumb className="block size-5 translate-x-0.5 rounded-full bg-background shadow transition-transform data-[checked]:translate-x-[22px]" />
    </Switch.Root>
  );
}
