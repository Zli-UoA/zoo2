import { useCall } from "../../lib/call/call-context";
import { saveSelectedDevice } from "../../lib/call/call-devices";
import { CallDeviceSelects } from "./CallDeviceSelects";

/** 設定の通話タブ: 既定のマイク/スピーカー。通話中なら即時に反映する。 */
export function CallSettingsDevices() {
  const { selectedMicId, selectedSpeakerId, setMicrophone, setSpeaker } = useCall();
  return (
    <CallDeviceSelects
      micId={selectedMicId}
      speakerId={selectedSpeakerId}
      onMicChange={(id) => {
        // CallProvider 外でも保存できるようにする
        saveSelectedDevice("mic", id);
        void setMicrophone(id);
      }}
      onSpeakerChange={(id) => {
        saveSelectedDevice("speaker", id);
        void setSpeaker(id);
      }}
    />
  );
}
