package qvtorun;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

/**
 * Batch mode: runs the cases of a case file in one JVM.
 *
 * <pre>
 * qvto-run --batch CASES --out DIR [--base DIR] [--only id,id,...] [--prefix p] [--no-tape]
 *          [--save-xmi] [--timeout SECONDS]
 * </pre>
 *
 * Paths in the case file are relative to --base (default: the current directory). Each case
 * runs on a fresh thread and writes DIR/&lt;id&gt;/ (see CaseRunner); DIR/results.tsv has one
 * row per case: id, verdict (pass, fail, error, timeout), load severity, execution severity,
 * code, exception class, checks, FNV-64 of out.jsonl, FNV-64 of log.txt, compile ms, run ms,
 * first failure.
 */
public final class Batch {
	private Batch() {}

	public static void main(String[] args) throws Exception {
		Path cases = null, out = null, base = Paths.get("");
		Set<String> only = null;
		List<String> prefixes = new ArrayList<>();
		boolean tape = true, saveXmi = false;
		long timeout = 120;
		for (int i = 0; i < args.length; i++) {
			switch (args[i]) {
			case "--batch": cases = Paths.get(args[++i]); break;
			case "--out": out = Paths.get(args[++i]); break;
			case "--base": base = Paths.get(args[++i]); break;
			case "--only": only = new HashSet<>(Arrays.asList(args[++i].split(","))); break;
			case "--prefix": prefixes.add(args[++i]); break;
			case "--no-tape": tape = false; break;
			case "--save-xmi": saveXmi = true; break;
			case "--timeout": timeout = Long.parseLong(args[++i]); break;
			default: throw new IllegalArgumentException("unknown argument " + args[i]);
			}
		}
		if (cases == null || out == null) throw new IllegalArgumentException("--batch CASES --out DIR required");
		Files.createDirectories(out);
		CaseRunner runner = new CaseRunner(base.toAbsolutePath(), out.toAbsolutePath());
		runner.tape = tape;
		runner.saveXmi = saveXmi;
		runner.timeoutMs = timeout * 1000;
		// the environment's set-up draws identity hashes: on this thread, not on the first case's
		// (whose tape seed must be the first two draws of its thread)
		Env.init();
		Tape.seed(); // loads the class here too
		List<Job> jobs = Job.read(cases);
		StringBuilder tsv = new StringBuilder("id\tverdict\tload\tseverity\tcode\texception\tchecks\tdump_fnv\tlog_fnv\tcompile_ms\trun_ms\tdetail\n");
		int pass = 0, n = 0;
		long t0 = System.nanoTime();
		for (Job j : jobs) {
			if (only != null && !only.contains(j.id)) continue;
			if (!prefixes.isEmpty() && prefixes.stream().noneMatch(j.id::startsWith)) continue;
			CaseRunner.Outcome o = runner.runIsolated(j);
			n++;
			if (o.verdict.equals("pass")) pass++;
			tsv.append(o.row()).append('\n');
			System.out.println(o.verdict + "\t" + j.id + (o.detail.isEmpty() ? "" : "\t" + o.detail));
		}
		Files.write(out.resolve("results.tsv"), tsv.toString().getBytes(StandardCharsets.UTF_8));
		System.out.printf("%d of %d cases pass (%.1f s)%n", pass, n, (System.nanoTime() - t0) / 1e9);
		System.exit(0);
	}
}
