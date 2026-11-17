-- Create index "sip_lines_created_by_idx" to table: "sip_lines"
CREATE INDEX "sip_lines_created_by_idx" ON "sip_lines" ("created_by");
-- Create index "sip_phones_line_extension_idx" to table: "sip_phones"
CREATE INDEX "sip_phones_line_extension_idx" ON "sip_phones" ("line_id", "extension");
