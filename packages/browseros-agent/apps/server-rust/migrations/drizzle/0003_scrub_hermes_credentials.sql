UPDATE `agent_definitions`
SET `adapter_config_json` = NULL
WHERE `adapter` = 'hermes' AND `adapter_config_json` IS NOT NULL;
