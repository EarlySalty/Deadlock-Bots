# Missing/invalid structure, unsuccessful invocations and warning/error findings fail.
def no_scanner_errors:
  (.invocations | type == "array" and length > 0) and
  all(.invocations[];
    .executionSuccessful == true and
    all(.toolExecutionNotifications[]?; .level != "error" and .level != "warning") and
    all(.toolConfigurationNotifications[]?; .level != "error" and .level != "warning"));
def security_severity($run; $result):
  [
    $result.properties."security-severity",
    ($run.tool.driver.rules[]? | select(.id == $result.ruleId) | .properties."security-severity"),
    ($run.tool.extensions[]?.rules[]? | select(.id == $result.ruleId) | .properties."security-severity")
  ] | map(select(. != null) | tonumber) | max // 0;
def no_findings:
  . as $run |
  (.results | type == "array") and
  all(.results[];
    . as $result |
    ((.level // "warning") == "note" or (.level // "warning") == "none") and
    security_severity($run; $result) < 7);
type == "object" and .version == "2.1.0" and
(.runs | type == "array" and length > 0) and
all(.runs[];
  .tool.driver.name == "CodeQL" and
  no_scanner_errors and no_findings)
