-- Remove schema tables that have no runtime readers or writers.
DROP TABLE IF EXISTS chat_messages;
DROP TABLE IF EXISTS chat_sessions;
DROP TABLE IF EXISTS llm_providers;
DROP TABLE IF EXISTS queries;
