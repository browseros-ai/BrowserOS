CREATE TABLE `recording_payloads` (
	`document_id` text PRIMARY KEY NOT NULL,
	`events_ndjson` text NOT NULL,
	FOREIGN KEY (`document_id`) REFERENCES `recording_streams`(`document_id`) ON UPDATE no action ON DELETE cascade
);
