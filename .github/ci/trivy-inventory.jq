# Used with jq --slurp: reject empty input and concatenated JSON documents.
# --list-all-pkgs supplies package inventories even when there are no findings.
def package_coordinates:
  type == "object"
  and (.Name | type) == "string" and (.Name | length) > 0
  and (.Version | type) == "string" and (.Version | length) > 0;

# Trivy 0.74.0 emits this nameless graph root for a Cargo workspace.
# It is not a package and cannot itself prove that dependencies were scanned.
def cargo_workspace_root:
  type == "object"
  and .Relationship == "root" and .AnalyzedBy == "cargo"
  and .Name == null and .Version == null
  and (.ID | type) == "string" and (.ID | length) > 0
  and (.Identifier.UID | type) == "string" and (.Identifier.UID | length) > 0
  and (.DependsOn | type) == "array" and (.DependsOn | length) > 0
  and all(.DependsOn[]; type == "string" and length > 0);

def language_target($target; $kind):
  [.Results[] | select(.Target == $target)] as $matches
  | ($matches | length) == 1
    and $matches[0].Class == "lang-pkgs"
    and $matches[0].Type == $kind
    and ($matches[0].Packages | type) == "array"
    and any($matches[0].Packages[]; package_coordinates)
    and all($matches[0].Packages[];
      package_coordinates or ($kind == "cargo" and cargo_workspace_root));

def docker_target:
  [.Results[] | select(.Target == ".clusterfuzzlite/Dockerfile")] as $matches
  | ($matches | length) == 1
    and $matches[0].Class == "config"
    and $matches[0].Type == "dockerfile"
    and ($matches[0].MisconfSummary | type) == "object"
    and all([$matches[0].MisconfSummary.Successes,
             $matches[0].MisconfSummary.Failures][];
      type == "number" and . >= 0 and floor == .)
    and (($matches[0].MisconfSummary.Successes // 0)
         + ($matches[0].MisconfSummary.Failures // 0)) > 0;

def valid_report:
  (type == "object")
  and .SchemaVersion == 2
  and (.Results | type) == "array"
  and language_target("rust/Cargo.lock"; "cargo")
  and language_target(".github/eslint-security/package-lock.json"; "npm")
  and language_target(".github/ci/python-requirements.txt"; "pip")
  and language_target(".github/ci/semgrep-requirements.txt"; "pip")
  and docker_target;

(type == "array") and length == 1 and (.[0] | valid_report)
