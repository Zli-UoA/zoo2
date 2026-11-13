import { BasicIndex, createCollection } from "@tanstack/react-db";
import { electricCollectionOptions } from "@tanstack/electric-db-collection";

import { joinUrl, withParams, type AppConfig } from "../config";
import { shapeAuthSource } from "../electric/collections";
import { createShapeFetch } from "../electric/headers";
import { resolveShapeUrl } from "../electric/query";
import type { ToDeviceRow } from "./protocol";

/**
 * 自分の端末あての to-device メッセージの Shape（`/api/shapes/e2ee_to_device?device_id=`）。
 *
 * サーバーは ack で行を消すので、Shape には未処理のメッセージだけが残る。永続化はしない
 * （処理済みの印は暗号ストアに置く）。
 */
export function toDeviceShapeUrl(config: AppConfig, userId: string, deviceId: string): string {
  if (config.shapeMode === "direct") {
    return withParams(config.electricShapeUrl, {
      table: "e2ee_to_device_messages",
      where: "recipient_user_id = $1 AND recipient_device_id = $2",
      "params[1]": userId,
      "params[2]": deviceId,
    });
  }
  return withParams(joinUrl(config.apiBaseUrl, `${config.shapeProxyPath}/e2ee_to_device`), {
    device_id: deviceId,
  });
}

interface ToDeviceShapeRow extends ToDeviceRow {
  recipient_user_id: string;
  recipient_device_id: string;
}

export interface ToDeviceSubscription {
  unsubscribe(): void;
}

/**
 * Shape を購読し、行が変わるたびに、今残っている行を ID の昇順で `onRows` に渡す。
 */
export function subscribeToDevice(
  config: AppConfig,
  userId: string,
  deviceId: string,
  onRows: (rows: ToDeviceRow[]) => void,
): ToDeviceSubscription {
  const collection = createCollection(
    electricCollectionOptions<ToDeviceShapeRow>({
      id: `e2ee_to_device:${deviceId}`,
      getKey: (row) => String(row.id),
      defaultIndexType: BasicIndex,
      shapeOptions: {
        url: resolveShapeUrl(toDeviceShapeUrl(config, userId, deviceId), window.location.href),
        fetchClient: createShapeFetch(shapeAuthSource(config)),
        liveSse: true,
      },
    }),
  );
  const emit = () => {
    const rows = [...collection.state.values()]
      .map((row) => ({
        id: String(row.id),
        sender_user_id: row.sender_user_id,
        sender_device_id: row.sender_device_id,
        type: row.type,
        content: row.content,
      }))
      .sort((a, b) => compareIds(a.id, b.id));
    if (rows.length > 0) onRows(rows);
  };
  const subscription = collection.subscribeChanges(emit, { includeInitialState: true });
  void collection.preload().catch(() => undefined);
  return {
    unsubscribe() {
      subscription.unsubscribe();
      void collection.cleanup().catch(() => undefined);
    },
  };
}

/** bigint の ID（文字列）を数として比べる。 */
export function compareIds(a: string, b: string): number {
  return a.length - b.length || (a < b ? -1 : a > b ? 1 : 0);
}
