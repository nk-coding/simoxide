package qvtorun;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * One case of a case file (the job list of the differential harness; written and documented by
 * crates/qvto-testkit, "Case files"). A case is a block of lines:
 *
 * <pre>
 * case &lt;id&gt;
 * dir &lt;path&gt;                  directory of the case's files (relative to the base directory)
 * qvto &lt;path&gt;                 transformation (paths relative to dir, else to the base directory)
 * in &lt;path&gt;                   model for the next in/inout parameter (positional)
 * expected &lt;path&gt;             expected model of the next out/inout parameter (positional)
 * mmfile &lt;path&gt;               .ecore loaded and registered in the resource set (includeMetamodelFile)
 * package &lt;nsURI&gt;             package of the global registry put into the resource set's registry
 * config &lt;key&gt;=&lt;value&gt;       configuration property (String value)
 * config-null &lt;key&gt;           configuration property with a null value
 * blackbox &lt;class&gt; [&lt;unit&gt; &lt;module&gt; [&lt;nsURI&gt;,...]]   BlackboxRegistry.registerModule(...)
 * blackbox-modules &lt;class&gt;    BlackboxRegistry.registerModules(class)
 * bundle-blackboxes            every blackbox the test bundle's plugin.xml declares (OSGi drivers)
 * session readonly-guard       QVTEvaluationOptions.FLAG_READONLY_GUARD_ENABLED = true
 * session max-stack-depth &lt;n&gt;
 * trace-in &lt;path&gt;             QVTEvaluationOptions.INCREMENTAL_UPDATE_TRACE from a .qvtotrace file
 * no-line-numbers              compile without source line numbers
 * urimap &lt;from&gt; &lt;to&gt;          URIConverter.URI_MAP entry for the case (to: a directory relative to the base)
 * imports folder               imports resolve relative to the transformation's folder (TestQvtParser's
 *                              workspace resolver), not against the bundle's source containers
 * check model                  outputs equal the expected models (the suite's comparer)
 * check severity &lt;n&gt; &lt;code&gt;   severity and code of the execution diagnostic
 * check log &lt;path&gt;             log equals the file
 * check log-nonempty-equal &lt;path&gt;  log equals the file, both non-empty
 * check stack &lt;path&gt;           printed QVT stack trace equals the file
 * check exception &lt;class&gt;      class of the QVT runtime exception
 * check no-exception           no QVT runtime exception
 * check trace-records &lt;n&gt;      number of trace records
 * check stack-frames &lt;n&gt; &lt;op&gt; &lt;module&gt; &lt;unit&gt; &lt;line&gt;|&lt;line&gt;...   the first n-1 frames
 * check problems &lt;e&gt; &lt;w&gt; [cst]  only compile; e errors and w warnings (-1: any) over the unit
 *                              and its imports (cst: only .qvto units), as TestQvtParser counts
 * check load-error             the transformation fails to load
 * unrunnable &lt;category&gt; &lt;why&gt;  not a job (classified as not runnable outside Eclipse)
 * end
 * </pre>
 */
final class Job {
	String id;
	String dir;
	String unrunnable;
	String qvto;
	final List<String> in = new ArrayList<>();
	final List<String> expected = new ArrayList<>();
	final List<String> mmfiles = new ArrayList<>();
	final List<String> packages = new ArrayList<>();
	final Map<String, String> config = new LinkedHashMap<>();
	final List<String[]> blackboxes = new ArrayList<>();
	final List<String> blackboxModules = new ArrayList<>();
	boolean bundleBlackboxes;
	boolean readonlyGuard;
	Integer maxStackDepth;
	String traceIn;
	boolean noLineNumbers;
	boolean importsFromFolder;
	final List<String[]> urimaps = new ArrayList<>();
	final List<String[]> checks = new ArrayList<>();

	boolean compileOnly() {
		for (String[] c : checks) if (c[0].equals("problems") || c[0].equals("load-error")) return true;
		return false;
	}

	/** A path of the case relative to the base directory. */
	String path(String p) {
		if (p.startsWith("/") || dir == null) return p;
		return dir + "/" + p;
	}

	static List<Job> read(Path file) throws IOException {
		List<Job> jobs = new ArrayList<>();
		Job cur = null;
		int n = 0;
		for (String raw : Files.readAllLines(file, StandardCharsets.UTF_8)) {
			n++;
			String line = raw.strip();
			if (line.isEmpty() || line.startsWith("#")) continue;
			int sp = line.indexOf(' ');
			String key = sp < 0 ? line : line.substring(0, sp);
			String rest = sp < 0 ? "" : line.substring(sp + 1);
			if (key.equals("case")) {
				if (cur != null) throw new IOException(file + ":" + n + ": missing end");
				cur = new Job();
				cur.id = rest;
				continue;
			}
			if (cur == null) {
				// lines outside cases (case-file metadata such as "base", "classify") are not jobs
				continue;
			}
			switch (key) {
			case "end": if (cur.unrunnable == null) jobs.add(cur); cur = null; break;
			case "dir": cur.dir = rest; break;
			case "unrunnable": cur.unrunnable = rest; break;
			case "qvto": cur.qvto = rest; break;
			case "in": cur.in.add(rest); break;
			case "expected": cur.expected.add(rest); break;
			case "mmfile": cur.mmfiles.add(rest); break;
			case "package": cur.packages.add(rest); break;
			case "config": {
				int eq = rest.indexOf('=');
				cur.config.put(rest.substring(0, eq), rest.substring(eq + 1));
				break;
			}
			case "config-null": cur.config.put(rest, null); break;
			case "blackbox": cur.blackboxes.add(rest.split(" ")); break;
			case "blackbox-modules": cur.blackboxModules.add(rest); break;
			case "bundle-blackboxes": cur.bundleBlackboxes = true; break;
			case "session": {
				String[] p = rest.split(" ");
				if (p[0].equals("readonly-guard")) cur.readonlyGuard = true;
				else if (p[0].equals("max-stack-depth")) cur.maxStackDepth = Integer.valueOf(p[1]);
				else throw new IOException(file + ":" + n + ": unknown session option " + p[0]);
				break;
			}
			case "trace-in": cur.traceIn = rest; break;
			case "no-line-numbers": cur.noLineNumbers = true; break;
			case "imports": cur.importsFromFolder = rest.equals("folder"); break;
			case "urimap": cur.urimaps.add(rest.split(" ", 2)); break;
			case "check": cur.checks.add(rest.split(" ")); break;
			default:
				// keys for the Rust side only (kind, driver, feature, source, ...)
				break;
			}
		}
		if (cur != null) throw new IOException(file + ": missing end after case " + cur.id);
		return jobs;
	}
}
