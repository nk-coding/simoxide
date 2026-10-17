# Shell helpers of the reference harnesses (refsim, the QVTo runner, the Slingshot runner). Sourced, POSIX
# sh (crates/qvto/reference/runner/qvto-run is a sh script). The Rust side is crates/simoxide-oraclekit.
#
#   oraclekit_exec_java ARG...         exec the JVM, niced: nice -n $ORACLE_NICE (default 19) ${JAVA:-java} ARG...
#   oraclekit_java ARG...              the same without exec
#   oraclekit_limited SECONDS CMD...   CMD under a time limit (timeout: exit 124) and a file-size limit
#                                      (RUN_FILE_LIMIT_KB, default 1 GB: SIGXFSZ, exit 153)
#   oraclekit_first_diff A B FILE...   prints the first FILE whose contents differ between directories A and B
#                                      (nothing if all are equal); B/FILE may be stored as B/FILE.gz; a file
#                                      missing on both sides is skipped, on one side it differs
#
# Each launcher keeps its own options (classpath, heap, agents) and its older variables for the niceness
# (QVTO_NICE, NICE), which it maps to ORACLE_NICE.

oraclekit_exec_java() {
  exec nice -n "${ORACLE_NICE:-19}" "${JAVA:-java}" "$@"
}

oraclekit_java() {
  nice -n "${ORACLE_NICE:-19}" "${JAVA:-java}" "$@"
}

oraclekit_limited() {
  _ok_t=$1; shift
  ( ulimit -f "${RUN_FILE_LIMIT_KB:-1048576}"; timeout "$_ok_t" "$@" )
}

oraclekit_first_diff() {
  _ok_a=$1; _ok_b=$2; shift 2
  for _ok_f in "$@"; do
    if [ ! -f "$_ok_b/$_ok_f" ] && [ -f "$_ok_b/$_ok_f.gz" ]; then
      gzip -dc "$_ok_b/$_ok_f.gz" | cmp -s - "$_ok_a/$_ok_f" || { echo "$_ok_f"; return 0; }
      continue
    fi
    [ -f "$_ok_a/$_ok_f" ] || [ -f "$_ok_b/$_ok_f" ] || continue
    cmp -s "$_ok_a/$_ok_f" "$_ok_b/$_ok_f" || { echo "$_ok_f"; return 0; }
  done
  return 0
}
