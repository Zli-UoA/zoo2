// Optional strict visual diagnostics only; normal runs verify operations and
// retain both renders for review. The same RGBA/opacity can differ between DOM and
// Canvas paint pipelines. Keep zero mismatching pixels beyond this tiny color
// distance; layout, text, state and larger color differences still fail.
export const composeVisualColorThreshold = 0.005;
