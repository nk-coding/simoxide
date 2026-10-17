package qvtorun;

import java.io.StringWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.emf.common.util.Diagnostic;
import org.eclipse.emf.common.util.TreeIterator;
import org.eclipse.emf.common.util.URI;
import org.eclipse.emf.ecore.EObject;
import org.eclipse.emf.ecore.EPackage;
import org.eclipse.emf.ecore.resource.Resource;
import org.eclipse.emf.ecore.resource.ResourceSet;
import org.eclipse.m2m.internal.qvt.oml.InternalTransformationExecutor;
import org.eclipse.m2m.internal.qvt.oml.expressions.DirectionKind;
import org.eclipse.m2m.internal.qvt.oml.expressions.ModelParameter;
import org.eclipse.m2m.qvt.oml.BasicModelExtent;
import org.eclipse.m2m.qvt.oml.ExecutionContextImpl;
import org.eclipse.m2m.qvt.oml.ExecutionDiagnostic;
import org.eclipse.m2m.qvt.oml.ModelExtent;
import org.eclipse.m2m.qvt.oml.util.WriterLog;

/**
 * Benchmark mode (the Java side of {@code qvto bench}, crates/qvto-cli/src/bench.rs): runs the
 * workloads of a workload file (case format with {@code bench <key> <value>} lines) in one JVM
 * and writes their compile and run times.
 *
 * <pre>
 * qvto-run --bench WORKLOADS --out RESULTS.tsv [--base DIR] [--only id,id,...] [--compiles K]
 *          [--runs N] [--budget-ms B]
 * </pre>
 *
 * Per workload: the transformation is compiled K times (a new executor each time, the time of
 * {@code loadTransformation}); the last executor then runs it. Fresh mode (default) loads the
 * inputs into a new resource set before every run (not timed) and runs until N runs or B ms of
 * run time (at least 5 runs); series mode ({@code bench series N}) loads them once and runs N
 * times on the same, changing models (as SimuLizar runs its rules during a simulation), in
 * {@code bench rounds} rounds (default 2) on freshly loaded models of which the last counts. The
 * time of a run is {@code execute(context, extents)}.
 *
 * RESULTS.tsv: id, compile times (ms, comma-separated), run times, severity and code of the last
 * run, objects in the out/inout extents after it, log lines of it.
 */
public final class Bench {
	private Bench() {}

	public static void main(String[] args) throws Exception {
		Path file = null, out = null, base = Paths.get("");
		Set<String> only = null;
		int compiles = 10, runs = 30;
		long budgetMs = 2000;
		for (int i = 0; i < args.length; i++) {
			switch (args[i]) {
			case "--bench": file = Paths.get(args[++i]); break;
			case "--out": out = Paths.get(args[++i]); break;
			case "--base": base = Paths.get(args[++i]); break;
			case "--only": only = new HashSet<>(Arrays.asList(args[++i].split(","))); break;
			case "--compiles": compiles = Integer.parseInt(args[++i]); break;
			case "--runs": runs = Integer.parseInt(args[++i]); break;
			case "--budget-ms": budgetMs = Long.parseLong(args[++i]); break;
			default: throw new IllegalArgumentException("unknown argument " + args[i]);
			}
		}
		if (file == null || out == null) throw new IllegalArgumentException("--bench FILE --out FILE required");
		Env.init();
		CaseRunner runner = new CaseRunner(base.toAbsolutePath(), base.toAbsolutePath());
		StringBuilder tsv = new StringBuilder("id\tcompile_ms\trun_ms\tseverity\tcode\tout_objects\tlog_lines\n");
		for (Job j : Job.read(file)) {
			if (only != null && !only.contains(j.id)) continue;
			final int k = compiles, n = runs;
			final long b = budgetMs;
			String[] row = new String[1];
			Thread t = new Thread(null, () -> {
				try {
					row[0] = run(runner, j, k, n, b);
				} catch (Throwable e) {
					row[0] = j.id + "\t-\t-\terror\t0\t0\t0\t" + e;
					e.printStackTrace();
				}
			}, "bench-" + j.id, 512L << 20);
			t.start();
			t.join();
			tsv.append(row[0]).append('\n');
			System.out.println(row[0].length() > 160 ? row[0].substring(0, 160) + "..." : row[0]);
		}
		Files.write(out, tsv.toString().getBytes(StandardCharsets.UTF_8));
		System.exit(0);
	}

	static String run(CaseRunner runner, Job job, int compiles, int maxRuns, long budgetMs) throws Exception {
		org.eclipse.m2m.internal.qvt.oml.blackbox.BlackboxRegistry.INSTANCE.cleanup();
		// the case's environment as CaseRunner sets it up (blackboxes, session options)
		List<Env.Bbox> bboxes = new ArrayList<>();
		if (job.bundleBlackboxes) bboxes.addAll(Env.bundleBlackboxes());
		for (String[] b : job.blackboxes) {
			String[] mms = b.length > 3 ? b[3].split(",") : null;
			bboxes.add(new Env.Bbox(b[0], b.length > 1 ? b[1] : null, b.length > 2 ? b[2] : null, mms));
		}
		for (Env.Bbox b : bboxes) Env.register(b);
		for (String c : job.blackboxModules) {
			org.eclipse.m2m.qvt.oml.TransformationExecutor.BlackboxRegistry.INSTANCE.registerModules(Class.forName(c));
		}
		BundleUnitResolverFactory.enabled = !job.importsFromFolder;
		BundleUnitResolverFactory.caseRoot = job.importRoot == null ? null
				: new BundleUnitResolverFactory.Bundle(runner.base.resolve(job.importRoot).toFile().getCanonicalFile(),
						java.util.Collections.singletonList(""));
		for (String[] m : job.urimaps) {
			String to = runner.base.resolve(m[1]).toAbsolutePath().toString();
			org.eclipse.emf.ecore.resource.URIConverter.URI_MAP.put(URI.createURI(m[0]), URI.createFileURI(to.endsWith("/") ? to : to + "/"));
		}
		if (job.conf != null) Env.applyConf(runner.base.resolve(job.conf), runner.base);
		URI transfUri = runner.fileUri(job.path(job.qvto));
		int series = job.bench.containsKey("series") ? Integer.parseInt(job.bench.get("series")) : 0;
		int rounds = series == 0 ? 1 : Math.max(1, job.bench.containsKey("rounds") ? Integer.parseInt(job.bench.get("rounds")) : 2);
		if (job.bench.containsKey("runs")) maxRuns = Integer.parseInt(job.bench.get("runs"));

		ResourceSet resSet = runner.metamodelResolutionRS(job);
		EPackage.Registry registry = resSet.getPackageRegistry();
		List<Double> compileMs = new ArrayList<>();
		InternalTransformationExecutor executor = null;
		for (int i = 0; i < Math.max(1, compiles); i++) {
			if (executor != null) executor.cleanup();
			long t0 = System.nanoTime();
			executor = new InternalTransformationExecutor(CaseRunner.transformation(transfUri, registry, job));
			Diagnostic d = executor.loadTransformation(new NullProgressMonitor());
			compileMs.add((System.nanoTime() - t0) / 1e6);
			if (d.getSeverity() > Diagnostic.WARNING) throw new IllegalStateException("load: " + d.getMessage());
		}
		List<DirectionKind> kinds = new ArrayList<>();
		for (ModelParameter p : executor.getTransformation().getModelParameter()) kinds.add(p.getKind());

		List<Double> runMs = new ArrayList<>();
		List<Resource> inputs = null;
		ExecutionDiagnostic ed = null;
		String log = "";
		List<BasicModelExtent> extents = null;
		// series: `rounds` rounds on freshly loaded models, the last one counts
		for (int round = 0; round < rounds; round++) {
		runMs.clear();
		inputs = null;
		double total = 0;
		int n = series > 0 ? series : maxRuns;
		for (int r = 0; r < n; r++) {
			if (series == 0 && r >= 5 && total > budgetMs) break;
			if (inputs == null || series == 0) {
				resSet = runner.metamodelResolutionRS(job);
				inputs = new ArrayList<>();
				int inIndex = 0;
				for (DirectionKind k : kinds) {
					Resource res = null;
					if (k != DirectionKind.OUT && inIndex < job.in.size()) {
						res = resSet.getResource(runner.fileUri(job.path(job.in.get(inIndex++))), true);
					}
					inputs.add(res);
				}
				org.eclipse.emf.ecore.util.EcoreUtil.resolveAll(resSet);
			}
			extents = new ArrayList<>();
			for (Resource res : inputs) extents.add(res == null ? new BasicModelExtent() : new BasicModelExtent(res.getContents()));
			ExecutionContextImpl ctx = new ExecutionContextImpl();
			for (Map.Entry<String, String> e : job.config.entrySet()) ctx.setConfigProperty(e.getKey(), e.getValue());
			if (job.readonlyGuard) ctx.getSessionData().setValue(org.eclipse.m2m.internal.qvt.oml.evaluator.QVTEvaluationOptions.FLAG_READONLY_GUARD_ENABLED, Boolean.TRUE);
			if (job.maxStackDepth != null) ctx.getSessionData().setValue(org.eclipse.m2m.internal.qvt.oml.evaluator.QVTEvaluationOptions.EVALUATION_MAX_STACK_DEPTH, job.maxStackDepth);
			if (job.traceIn != null) {
				Resource tr = resSet.getResource(runner.fileUri(job.path(job.traceIn)), true);
				ctx.getSessionData().setValue(org.eclipse.m2m.internal.qvt.oml.evaluator.QVTEvaluationOptions.INCREMENTAL_UPDATE_TRACE,
						new org.eclipse.m2m.qvt.oml.util.Trace(tr.getContents()));
			}
			StringWriter logBuf = new StringWriter();
			ctx.setLog(new WriterLog(logBuf, true));
			long t1 = System.nanoTime();
			ed = executor.execute(ctx, extents.toArray(new ModelExtent[0]));
			double dt = (System.nanoTime() - t1) / 1e6;
			runMs.add(dt);
			total += dt;
			log = logBuf.toString();
		}
		}
		int objects = 0;
		for (int i = 0; i < kinds.size(); i++) {
			if (kinds.get(i) == DirectionKind.IN) continue;
			for (EObject o : extents.get(i).getContents()) {
				objects++;
				for (TreeIterator<EObject> it = o.eAllContents(); it.hasNext(); it.next()) objects++;
			}
		}
		executor.cleanup();
		org.eclipse.m2m.internal.qvt.oml.blackbox.BlackboxRegistry.INSTANCE.cleanup();
		int logLines = log.isEmpty() ? 0 : log.split("\r?\n", -1).length - (log.endsWith("\n") ? 1 : 0);
		return job.id + "\t" + join(compileMs) + "\t" + join(runMs) + "\t" + Tape.sev(ed.getSeverity()) + "\t" + ed.getCode() + "\t" + objects
				+ "\t" + logLines;
	}

	static String join(List<Double> v) {
		StringBuilder sb = new StringBuilder();
		for (double d : v) sb.append(sb.length() > 0 ? "," : "").append(String.format(java.util.Locale.ROOT, "%.4f", d));
		return sb.toString();
	}
}
