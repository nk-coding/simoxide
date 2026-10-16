package qvtorun;

import java.io.File;
import java.io.IOException;
import java.io.PrintWriter;
import java.io.StringWriter;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import org.eclipse.core.runtime.IProgressMonitor;
import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.emf.common.util.Diagnostic;
import org.eclipse.emf.common.util.URI;
import org.eclipse.emf.ecore.EObject;
import org.eclipse.emf.ecore.EPackage;
import org.eclipse.emf.ecore.EcorePackage;
import org.eclipse.emf.ecore.resource.Resource;
import org.eclipse.emf.ecore.resource.ResourceSet;
import org.eclipse.emf.ecore.resource.impl.ResourceSetImpl;
import org.eclipse.emf.ecore.xmi.XMLResource;
import org.eclipse.emf.ecore.xmi.impl.EcoreResourceFactoryImpl;
import org.eclipse.emf.ecore.xmi.impl.XMIResourceFactoryImpl;
import org.eclipse.m2m.internal.qvt.oml.InternalTransformationExecutor;
import org.eclipse.m2m.internal.qvt.oml.QvtMessage;
import org.eclipse.m2m.internal.qvt.oml.compiler.CompiledUnit;
import org.eclipse.m2m.internal.qvt.oml.compiler.PlatformNamespaceUriResourceMap;
import org.eclipse.m2m.internal.qvt.oml.compiler.QVTOCompiler;
import org.eclipse.m2m.internal.qvt.oml.compiler.QvtCompilerOptions;
import org.eclipse.m2m.internal.qvt.oml.compiler.UnitProxy;
import org.eclipse.m2m.internal.qvt.oml.evaluator.QVTEvaluationOptions;
import org.eclipse.m2m.internal.qvt.oml.evaluator.EvaluationMessages;
import org.eclipse.m2m.internal.qvt.oml.expressions.DirectionKind;
import org.eclipse.m2m.internal.qvt.oml.expressions.ModelParameter;
import org.eclipse.m2m.internal.qvt.oml.expressions.OperationalTransformation;
import org.eclipse.m2m.internal.qvt.oml.trace.Trace;
import org.eclipse.m2m.internal.qvt.oml.CstTransformation;
import org.eclipse.m2m.qvt.oml.BasicModelExtent;
import org.eclipse.m2m.qvt.oml.ExecutionContextImpl;
import org.eclipse.m2m.qvt.oml.ExecutionDiagnostic;
import org.eclipse.m2m.qvt.oml.ExecutionStackTraceElement;
import org.eclipse.m2m.qvt.oml.ModelExtent;
import org.eclipse.m2m.qvt.oml.util.WriterLog;

/**
 * Runs one case the way the Eclipse suite's TestQvtStandaloneExecutor does (resource set,
 * metamodel registrations, parameter binding, blackbox registration, comparison of the outputs),
 * with the extras of the other drivers (log, QVT stack trace, trace records, session options),
 * and writes everything observable into the case's result directory:
 *
 * <pre>
 * result.txt   key/value lines: load, severity, code, message, exception, params, checks, verdict
 * log.txt      the transformation's log (WriterLog)
 * stack.txt    the printed QVT stack trace of the terminating exception (if any)
 * diag.txt     load problems (severity, line, offset, length, message, unit) and the execution
 *              diagnostic tree
 * out.jsonl    canonical dump (CanonDump) of the out/inout extents, each in a resource
 *              "extent&lt;i&gt;.xmi" (i = parameter index)
 * idtape.tsv   identity-hash tape (see Tape)
 * </pre>
 */
final class CaseRunner {
	final Path base;
	final Path outRoot;
	boolean tape = true;
	boolean saveXmi;
	long timeoutMs = 120_000;
	/** URI_MAP entries of the previous case (removed before the next one). */
	private final List<URI> caseUriMaps = new ArrayList<>();

	CaseRunner(Path base, Path outRoot) {
		this.base = base;
		this.outRoot = outRoot;
	}

	/** Result of one case: the summary row of results.tsv. */
	static final class Outcome {
		String id;
		String verdict = "error";
		String load = "-";
		String severity = "-";
		int code;
		String exception = "-";
		String checks = "";
		String detail = "";
		String dumpHash = "-";
		String logHash = "-";
		double compileMs, runMs;

		String row() {
			return id + "\t" + verdict + "\t" + load + "\t" + severity + "\t" + code + "\t" + exception + "\t" + checks + "\t"
					+ dumpHash + "\t" + logHash + "\t" + String.format("%.1f", compileMs) + "\t" + String.format("%.1f", runMs)
					+ "\t" + detail.replace('\t', ' ').replace('\n', ' ');
		}
	}

	/** Runs the case on a fresh thread (fresh identity-hash state) with a deadline. */
	Outcome runIsolated(Job job) {
		Outcome[] res = new Outcome[1];
		DeadlineMonitor monitor = new DeadlineMonitor(System.currentTimeMillis() + timeoutMs);
		Thread t = new Thread(() -> {
			try {
				res[0] = run(job, monitor);
			} catch (Throwable e) {
				Outcome o = new Outcome();
				o.id = job.id;
				o.verdict = "error";
				o.detail = "runner: " + e;
				StringWriter sw = new StringWriter();
				e.printStackTrace(new PrintWriter(sw));
				try {
					Path dir = outRoot.resolve(job.id);
					Files.createDirectories(dir);
					write(dir.resolve("runner-error.txt"), sw.toString());
				} catch (IOException x) {
					// ignore
				}
				res[0] = o;
			}
		}, "case-" + job.id);
		t.setDaemon(true);
		t.start();
		try {
			t.join(timeoutMs + 5_000);
			if (t.isAlive()) {
				monitor.cancel = true;
				t.join(10_000);
			}
		} catch (InterruptedException e) {
			Thread.currentThread().interrupt();
		}
		if (res[0] == null) {
			Outcome o = new Outcome();
			o.id = job.id;
			o.verdict = "timeout";
			o.detail = "no result after " + timeoutMs + " ms";
			return o;
		}
		return res[0];
	}

	/** Progress monitor cancelled at a deadline: QVTo checks it and interrupts the execution. */
	static final class DeadlineMonitor extends NullProgressMonitor {
		final long deadline;
		volatile boolean cancel;

		DeadlineMonitor(long deadline) {
			this.deadline = deadline;
		}

		@Override
		public boolean isCanceled() {
			return cancel || super.isCanceled() || System.currentTimeMillis() > deadline;
		}
	}

	/** A WriterLog that also keeps the message logged after "Terminating execution...". */
	static final class CapturingLog extends WriterLog {
		boolean terminating;
		String stack;

		CapturingLog(StringWriter w) {
			super(w, true);
		}

		@Override
		public void log(String message) {
			if (terminating && stack == null) stack = message;
			terminating = EvaluationMessages.TerminatingExecution.equals(message);
			super.log(message);
		}
	}

	Outcome run(Job job, IProgressMonitor monitor) throws Exception {
		Env.init();
		Outcome o = new Outcome();
		o.id = job.id;
		Path dir = outRoot.resolve(job.id);
		Files.createDirectories(dir);
		Map<String, String> result = new LinkedHashMap<>();
		result.put("id", job.id);
		int[] seed = Tape.seed();
		StringBuilder diag = new StringBuilder();

		org.eclipse.m2m.internal.qvt.oml.blackbox.BlackboxRegistry.INSTANCE.cleanup();
		List<Env.Bbox> bboxes = new ArrayList<>();
		if (job.bundleBlackboxes) bboxes.addAll(Env.bundleBlackboxes());
		for (String[] b : job.blackboxes) {
			String[] mms = b.length > 3 ? b[3].split(",") : null;
			bboxes.add(new Env.Bbox(b[0], b.length > 1 ? b[1] : null, b.length > 2 ? b[2] : null, mms));
		}
		for (Env.Bbox b : bboxes) {
			Diagnostic d = Env.register(b);
			if (d.getSeverity() > Diagnostic.WARNING) diag.append("blackbox ").append(b.cls).append(": ").append(d.getMessage()).append('\n');
		}
		for (String c : job.blackboxModules) {
			Diagnostic d = org.eclipse.m2m.qvt.oml.TransformationExecutor.BlackboxRegistry.INSTANCE.registerModules(Class.forName(c));
			if (d.getSeverity() > Diagnostic.WARNING) diag.append("blackbox ").append(c).append(": ").append(d.getMessage()).append('\n');
		}

		BundleUnitResolverFactory.enabled = !job.importsFromFolder;
		BundleUnitResolverFactory.caseRoot = job.importRoot == null ? null
				: new BundleUnitResolverFactory.Bundle(base.resolve(job.importRoot).toFile().getCanonicalFile(), java.util.Collections.singletonList(""));
		for (URI k : caseUriMaps) org.eclipse.emf.ecore.resource.URIConverter.URI_MAP.remove(k);
		caseUriMaps.clear();
		for (String[] m : job.urimaps) {
			caseUriMaps.add(URI.createURI(m[0]));
			String to = base.resolve(m[1]).toAbsolutePath().toString();
			org.eclipse.emf.ecore.resource.URIConverter.URI_MAP.put(URI.createURI(m[0]), URI.createFileURI(to.endsWith("/") ? to : to + "/"));
		}
		if (job.conf != null) Env.applyConf(base.resolve(job.conf), base);
		URI transfUri = fileUri(job.path(job.qvto));
		ResourceSet resSet = metamodelResolutionRS(job);
		EPackage.Registry registry = resSet.getPackageRegistry();

		// Parameter kinds, as TransformationExecutorTest.getParamKinds (a separate load).
		long t0 = System.nanoTime();
		List<DirectionKind> kinds = new ArrayList<>();
		List<String> names = new ArrayList<>();
		CompiledUnit[] unit = new CompiledUnit[1];
		{
			InternalTransformationExecutor k = new InternalTransformationExecutor(transformation(transfUri, registry, job));
			Diagnostic ld = k.loadTransformation(new NullProgressMonitor());
			o.load = Tape.sev(ld.getSeverity());
			result.put("load", o.load);
			unit[0] = k.getUnit();
			problems(unit[0], diag, new ArrayList<>());
			diagTree(ld, "load", diag);
			if (ld.getSeverity() < Diagnostic.ERROR && k.getTransformation() != null) {
				OperationalTransformation tr = k.getTransformation();
				for (ModelParameter p : tr.getModelParameter()) {
					kinds.add(p.getKind());
					names.add(p.getName());
				}
			}
			k.cleanup();
		}
		o.compileMs = (System.nanoTime() - t0) / 1e6;
		StringBuilder params = new StringBuilder();
		for (int i = 0; i < kinds.size(); i++) params.append(i > 0 ? "," : "").append(kinds.get(i).getLiteral()).append(':').append(names.get(i));
		result.put("params", params.toString());

		if (job.compileOnly()) {
			List<String> failures = new ArrayList<>();
			StringBuilder checks = new StringBuilder();
			for (String[] c : job.checks) {
				String fail = null;
				if (c[0].equals("load-error")) {
					if (!o.load.equals("ERROR")) fail = "the transformation loads (" + o.load + ")";
				} else if (c[0].equals("problems")) {
					boolean cst = c.length > 3 && c[3].equals("cst");
					int[] n = new int[2];
					count(unit[0], n, cst);
					int e = Integer.parseInt(c[1]), w = Integer.parseInt(c[2]);
					result.put("errors", String.valueOf(n[0]));
					result.put("warnings", String.valueOf(n[1]));
					if (unit[0] == null) fail = "no compiled unit";
					else if (e != n[0] || (w != -1 && w != n[1])) fail = "expected " + e + " errors, " + w + " warnings, got " + n[0] + ", " + n[1];
				} else {
					fail = "check " + c[0] + " needs an execution";
				}
				checks.append(checks.length() > 0 ? "," : "").append(c[0]).append(':').append(fail == null ? "pass" : "fail");
				result.put("check " + String.join(" ", c), fail == null ? "pass" : "fail: " + oneLine(fail));
				if (fail != null) failures.add(c[0] + ": " + fail);
			}
			o.checks = checks.toString();
			o.verdict = failures.isEmpty() ? "pass" : "fail";
			o.detail = failures.isEmpty() ? "" : failures.get(0);
			result.put("verdict", o.verdict);
			finish(dir, result, diag, "", null, o);
			org.eclipse.m2m.internal.qvt.oml.blackbox.BlackboxRegistry.INSTANCE.cleanup();
			return o;
		}

		// Extents (TransformationExecutorTest.setUp).
		List<BasicModelExtent> extents = new ArrayList<>();
		List<Resource> inputResources = new ArrayList<>();
		int inIndex = 0;
		for (int i = 0; i < kinds.size(); i++) {
			BasicModelExtent extent = new BasicModelExtent();
			if (kinds.get(i) != DirectionKind.OUT && inIndex < job.in.size()) {
				Resource r = resSet.getResource(fileUri(job.path(job.in.get(inIndex++))), true);
				inputResources.add(r);
				extent = new BasicModelExtent(r.getContents());
			}
			extents.add(extent);
		}

		InternalTransformationExecutor.TracesAwareExecutor executor =
				new InternalTransformationExecutor.TracesAwareExecutor(transformation(transfUri, registry, job));
		ExecutionContextImpl ctx = new ExecutionContextImpl();
		for (Map.Entry<String, String> e : job.config.entrySet()) ctx.setConfigProperty(e.getKey(), e.getValue());
		StringWriter logBuf = new StringWriter();
		CapturingLog log = new CapturingLog(logBuf);
		ctx.setLog(log);
		ctx.setProgressMonitor(monitor);
		if (job.readonlyGuard) ctx.getSessionData().setValue(QVTEvaluationOptions.FLAG_READONLY_GUARD_ENABLED, Boolean.TRUE);
		if (job.maxStackDepth != null) ctx.getSessionData().setValue(QVTEvaluationOptions.EVALUATION_MAX_STACK_DEPTH, job.maxStackDepth);
		if (job.traceIn != null) {
			Resource tr = resSet.getResource(fileUri(job.path(job.traceIn)), true);
			ctx.getSessionData().setValue(QVTEvaluationOptions.INCREMENTAL_UPDATE_TRACE,
					new org.eclipse.m2m.qvt.oml.util.Trace(tr.getContents()));
		}

		long t1 = System.nanoTime();
		ExecutionDiagnostic ed = executor.execute(ctx, extents.toArray(new ModelExtent[0]));
		o.runMs = (System.nanoTime() - t1) / 1e6;
		Tape.Snapshot snap = null;
		if (tape) {
			List<List<EObject>> outLists = new ArrayList<>();
			for (int i = 0; i < kinds.size(); i++) if (kinds.get(i) != DirectionKind.IN) outLists.add(extents.get(i).getContents());
			snap = Tape.snapshot(inputResources, outLists);
		}
		o.severity = Tape.sev(ed.getSeverity());
		o.code = ed.getCode();
		result.put("severity", o.severity);
		result.put("code", String.valueOf(o.code));
		result.put("message", oneLine(ed.getMessage()));
		diagTree(ed, "exec", diag);
		String stack = null;
		if (log.stack != null) {
			// the evaluator logs e.printQvtStackTrace() + line separator
			stack = log.stack.endsWith(System.lineSeparator()) ? log.stack.substring(0, log.stack.length() - System.lineSeparator().length()) : log.stack;
			String first = stack.split("\r?\n", 2)[0];
			int colon = first.indexOf(": ");
			o.exception = colon < 0 ? first : first.substring(0, colon);
		} else if (ed.getStackTrace() != null && !ed.getStackTrace().isEmpty()) {
			StringWriter sw = new StringWriter();
			ed.printStackTrace(new PrintWriter(sw));
			stack = sw.toString();
		}
		if (o.exception.equals("-") && ed.getData() != null && !ed.getData().isEmpty() && ed.getData().get(0) instanceof Throwable) {
			o.exception = ed.getData().get(0).getClass().getName();
		}
		result.put("exception", o.exception);
		Trace traces = executor.getTraces();
		int traceRecords = traces == null ? -1 : traces.getTraceRecords().size();
		result.put("trace-records", String.valueOf(traceRecords));

		// Checks (the drivers' assertions).
		List<String> failures = new ArrayList<>();
		StringBuilder checks = new StringBuilder();
		String logText = logBuf.toString();
		for (String[] c : job.checks) {
			String fail = check(c, job, resSet, kinds, extents, ed, logText, stack, o.exception, traceRecords);
			checks.append(checks.length() > 0 ? "," : "").append(c[0]).append(':').append(fail == null ? "pass" : "fail");
			result.put("check " + String.join(" ", c), fail == null ? "pass" : "fail: " + oneLine(fail));
			if (fail != null) failures.add(c[0] + ": " + fail);
		}
		o.checks = checks.toString();
		o.verdict = failures.isEmpty() ? "pass" : "fail";
		o.detail = failures.isEmpty() ? "" : failures.get(0);
		result.put("verdict", o.verdict);
		result.put("run-ms", String.format("%.3f", o.runMs));

		// Canonical dump of the out and inout extents. Proxies are resolved first (in the case's
		// resource set, whatever the run happened to resolve), as qvto-rs resolves eagerly.
		try {
			org.eclipse.emf.ecore.util.EcoreUtil.resolveAll(resSet);
			for (int i = 0; i < kinds.size(); i++) {
				if (kinds.get(i) == DirectionKind.IN) continue;
				for (EObject e : extents.get(i).getContents()) org.eclipse.emf.ecore.util.EcoreUtil.resolveAll(e);
			}
		} catch (RuntimeException e) {
			diag.append("resolve\t").append(oneLine(e.toString())).append('\n');
		}
		ResourceSet dumpRs = new ResourceSetImpl();
		List<Resource> outs = new ArrayList<>();
		for (int i = 0; i < kinds.size(); i++) {
			if (kinds.get(i) == DirectionKind.IN) continue;
			Resource r = dumpRs.createResource(URI.createFileURI(dir.resolve("extent" + i + ".xmi").toAbsolutePath().toString()));
			r.getContents().addAll(new ArrayList<>(extents.get(i).getContents()));
			outs.add(r);
		}
		List<Resource> dumped = new ArrayList<>(outs);
		if (job.dumpModels) {
			List<URI> mmUris0 = new ArrayList<>();
			for (String mm : job.mmfiles) mmUris0.add(fileUri(job.path(mm)));
			List<Resource> others = new ArrayList<>();
			for (Resource r : resSet.getResources()) {
				if (r.getURI() != null && r.getURI().isFile() && !mmUris0.contains(r.getURI())
						&& !"ecore".equals(r.getURI().fileExtension())) others.add(r);
			}
			others.sort((a, b) -> a.getURI().lastSegment().compareTo(b.getURI().lastSegment()));
			dumped.addAll(others);
		}
		String dump = canonDump(dumped);
		write(dir.resolve("out.jsonl"), dump);
		o.dumpHash = Tape.fnv(dump.getBytes(StandardCharsets.UTF_8));
		if (saveXmi) {
			Map<Object, Object> opts = new HashMap<>();
			opts.put(XMLResource.OPTION_ENCODING, "UTF-8");
			opts.put(XMLResource.OPTION_PROCESS_DANGLING_HREF, XMLResource.OPTION_PROCESS_DANGLING_HREF_RECORD);
			// the inputs as the run left them (inout changes, cross-resource containment), under their
			// own file names so that references keep their labels
			Path inDir = dir.resolve("inputs");
			Files.createDirectories(inDir);
			List<Resource> models = new ArrayList<>();
			List<URI> mmUris = new ArrayList<>();
			for (String mm : job.mmfiles) mmUris.add(fileUri(job.path(mm)));
			for (Resource r : new ArrayList<>(resSet.getResources())) {
				if (r.getURI() != null && r.getURI().isFile() && !mmUris.contains(r.getURI())) models.add(r);
			}
			for (Resource r : models) {
				r.setURI(URI.createFileURI(inDir.resolve(r.getURI().lastSegment()).toAbsolutePath().toString()));
			}
			for (Resource r : models) {
				try {
					r.save(opts);
				} catch (IOException e) {
					diag.append("save\t").append(oneLine(e.toString())).append('\n');
				}
			}
			for (Resource r : outs) {
				try {
					r.save(opts);
				} catch (IOException e) {
					diag.append("save\t").append(oneLine(e.toString())).append('\n');
				}
			}
		}
		if (snap != null) Tape.write(dir.resolve("idtape.tsv"), seed, snap);
		finish(dir, result, diag, logText, stack, o);
		executor.cleanup();
		org.eclipse.m2m.internal.qvt.oml.blackbox.BlackboxRegistry.INSTANCE.cleanup();
		return o;
	}

	private void finish(Path dir, Map<String, String> result, StringBuilder diag, String logText, String stack, Outcome o) throws IOException {
		logText = norm(logText);
		o.logHash = Tape.fnv(logText.getBytes(StandardCharsets.UTF_8));
		o.detail = norm(o.detail);
		StringBuilder sb = new StringBuilder();
		for (Map.Entry<String, String> e : result.entrySet()) sb.append(e.getKey()).append('\t').append(e.getValue()).append('\n');
		write(dir.resolve("result.txt"), norm(sb.toString()));
		write(dir.resolve("log.txt"), logText);
		write(dir.resolve("diag.txt"), norm(diag.toString()));
		if (stack != null) write(dir.resolve("stack.txt"), norm(stack));
		else Files.deleteIfExists(dir.resolve("stack.txt"));
	}

	private String check(String[] c, Job job, ResourceSet resSet, List<DirectionKind> kinds, List<BasicModelExtent> extents,
			ExecutionDiagnostic ed, String log, String stack, String exception, int traceRecords) throws Exception {
		switch (c[0]) {
		case "severity": {
			int sev = Integer.parseInt(c[1]), code = Integer.parseInt(c[2]);
			if (sev != ed.getSeverity() || code != ed.getCode()) {
				return "expected severity " + sev + " code " + code + ", got " + ed.getSeverity() + " code " + ed.getCode() + ": "
						+ ed.getMessage();
			}
			return null;
		}
		case "model": {
			// TestQvtExecutor.runTest: checking resource set with the registrations of resSet.
			ResourceSetImpl checking = new ResourceSetImpl();
			checking.setPackageRegistry(resSet.getPackageRegistry());
			if (resSet instanceof ResourceSetImpl) {
				checking.setURIResourceMap(((ResourceSetImpl) resSet).getURIResourceMap());
				checking.setResourceFactoryRegistry(resSet.getResourceFactoryRegistry());
			}
			int outIndex = 0;
			for (int i = 0; i < kinds.size(); i++) {
				if (kinds.get(i) == DirectionKind.IN) continue;
				if (outIndex >= job.expected.size()) return "no expected model for parameter " + i;
				Resource exp = checking.getResource(fileUri(job.path(job.expected.get(outIndex++))), true);
				List<EObject> actual = extents.get(i).getContents();
				if (exp.getContents().size() != actual.size()) {
					return "parameter " + i + ": output differs in size to expected result: expected " + exp.getContents().size() + ", got "
							+ actual.size();
				}
				if (!suiteEqual(exp.getContents(), actual)) return "parameter " + i + ": differs from " + job.expected.get(outIndex - 1);
			}
			return null;
		}
		case "log":
		case "log-nonempty-equal": {
			String exp = read(job.path(c[1]));
			if (c[0].equals("log-nonempty-equal") && (exp.isEmpty() || log.isEmpty())) return "empty log";
			return exp.equals(log) ? null : "log differs (" + firstDiff(exp, log) + ")";
		}
		case "stack": {
			String exp = read(job.path(c[1]));
			if (stack == null) return "no stack trace";
			return exp.equals(stack) ? null : "stack differs (" + firstDiff(exp, stack) + ")";
		}
		case "exception":
			return c[1].equals(exception) ? null : "expected exception " + c[1] + ", got " + exception;
		case "no-exception":
			return exception.equals("-") ? null : "unexpected exception " + exception;
		case "trace-records":
			return Integer.parseInt(c[1]) == traceRecords ? null : "expected " + c[1] + " trace records, got " + traceRecords;
		case "stack-frames": {
			// stack-frames <n> <op> <module> <unit> <line>|<line>
			List<ExecutionStackTraceElement> st = ed.getStackTrace();
			if (st == null || st.isEmpty()) return "no stack trace";
			int n = Integer.parseInt(c[1]);
			int count = n;
			for (ExecutionStackTraceElement e : st) {
				if (--count == 0) break;
				boolean lineOk = false;
				for (String l : c[5].split("\\|")) lineOk |= Integer.parseInt(l) == e.getLineNumber();
				if (!c[2].equals(e.getOperationName()) || !c[3].equals(e.getModuleName()) || !c[4].equals(e.getUnitName()) || !lineOk) {
					return "frame " + e;
				}
			}
			return null;
		}
		default:
			return "unknown check " + c[0];
		}
	}

	// --- resource sets and transformations --------------------------------------------------

	/** TestQvtStandaloneExecutor.createResourceSet + getMetamodelResolutionRS. */
	ResourceSet metamodelResolutionRS(Job job) {
		ResourceSetImpl rs = new ResourceSetImpl();
		rs.getResourceFactoryRegistry().getExtensionToFactoryMap().put("*", new EcoreResourceFactoryImpl());
		EPackage.Registry reg = rs.getPackageRegistry();
		if (!job.mmfiles.isEmpty()) {
			for (String mm : job.mmfiles) {
				Resource r = rs.getResource(fileUri(job.path(mm)), true);
				EPackage p = r.getContents().isEmpty() || !(r.getContents().get(0) instanceof EPackage) ? null : (EPackage) r.getContents().get(0);
				if (p == null) throw new IllegalStateException("No metamodel EPackage available in " + mm);
				if (!reg.containsKey(p.getNsURI())) reg.put(p.getNsURI(), p);
			}
			rs.setURIResourceMap(new PlatformNamespaceUriResourceMap(rs));
		}
		for (String m : job.in) rs.getResourceFactoryRegistry().getExtensionToFactoryMap().put(ext(m), new XMIResourceFactoryImpl());
		for (String m : job.expected) rs.getResourceFactoryRegistry().getExtensionToFactoryMap().put(ext(m), new XMIResourceFactoryImpl());
		for (String ns : job.packages) {
			EPackage p = EPackage.Registry.INSTANCE.getEPackage(ns);
			if (p == null) throw new IllegalStateException("package not registered: " + ns);
			reg.put(ns, p);
		}
		Map<URI, Resource> m = rs.getURIResourceMap();
		if (m != null) {
			URI ecoreResourceUri = URI.createPlatformResourceURI("org.eclipse.emf.ecore/model/Ecore.ecore", true);
			m.put(ecoreResourceUri, reg.getEPackage(EcorePackage.eNS_URI).eResource());
		}
		return rs;
	}

	static String ext(String path) {
		return path.substring(path.lastIndexOf('.') + 1);
	}

	/** The transformation, compiled with or without source line numbers. */
	static CstTransformation transformation(URI uri, EPackage.Registry registry, Job job) {
		if (!job.noLineNumbers) return new CstTransformation(uri, registry);
		return new CstTransformation(uri, registry) {
			@Override
			protected QVTOCompiler createCompiler() {
				return new QVTOCompiler(registry) {
					@Override
					public CompiledUnit[] compile(UnitProxy[] sources, QvtCompilerOptions options, IProgressMonitor monitor)
							throws org.eclipse.m2m.internal.qvt.oml.common.MdaException {
						QvtCompilerOptions o = options != null ? options : new QvtCompilerOptions();
						o.setGenerateCompletionData(false);
						o.setSourceLineNumbersEnabled(false);
						return super.compile(sources, o, monitor);
					}
				};
			}
		};
	}

	private List<String[]> prefixes;

	/**
	 * Machine-independent text: absolute paths of the base directory (as given and resolved, as
	 * paths and as file URIs) become "${root}".
	 */
	String norm(String s) {
		if (prefixes == null) {
			prefixes = new ArrayList<>();
			List<String> roots = new ArrayList<>();
			try {
				// symlinked parts of the base (e.g. a worktree's reference checkout) by their real paths
				for (String sub : new String[] { "reference/eclipse-qvto", "reference/eclipse-ocl", "" }) {
					Path p = base.resolve(sub);
					if (Files.exists(p)) {
						String real = p.toRealPath().toString();
						String abs = p.toAbsolutePath().normalize().toString();
						String to = "${root}" + (sub.isEmpty() ? "" : "/" + sub);
						if (!real.equals(abs)) prefixes.add(new String[] { real, to });
						prefixes.add(new String[] { abs, to });
					}
				}
			} catch (IOException e) {
				// keep what we have
			}
			List<String[]> withUris = new ArrayList<>();
			for (String[] p : prefixes) {
				withUris.add(new String[] { URI.createFileURI(p[0]).toString(), "file:" + p[1] });
				withUris.add(p);
			}
			prefixes = withUris;
		}
		for (String[] p : prefixes) s = s.replace(p[0], p[1]);
		return s;
	}

	URI fileUri(String path) {
		File f = new File(path);
		if (!f.isAbsolute()) f = base.resolve(path).normalize().toFile();
		return URI.createFileURI(f.getAbsolutePath());
	}

	String read(String path) throws IOException {
		Path p = base.resolve(path);
		return new String(Files.readAllBytes(p), StandardCharsets.UTF_8);
	}

	// --- diagnostics -------------------------------------------------------------------------

	static void problems(CompiledUnit unit, StringBuilder out, List<CompiledUnit> seen) {
		if (unit == null || seen.contains(unit)) return;
		seen.add(unit);
		String name = unit.getURI() == null ? "?" : unit.getURI().lastSegment();
		for (QvtMessage m : unit.getProblems()) {
			out.append("problem\t").append(m.getSeverity() == QvtMessage.SEVERITY_ERROR ? "error" : "warning").append('\t').append(name)
					.append('\t').append(m.getLineNum()).append('\t').append(m.getOffset()).append('\t').append(m.getLength()).append('\t')
					.append(oneLine(m.getMessage())).append('\n');
		}
		for (CompiledUnit i : unit.getCompiledImports()) problems(i, out, seen);
	}

	/** Errors and warnings of a unit and its imports, as TransformationUtil.getErrors counts them. */
	static void count(CompiledUnit unit, int[] n, boolean cstOnly) {
		if (unit == null) return;
		if (cstOnly && !"qvto".equals(unit.getURI().fileExtension())) return;
		n[0] += unit.getErrors().size();
		n[1] += unit.getWarnings().size();
		for (CompiledUnit i : unit.getCompiledImports()) count(i, n, cstOnly);
	}

	static void diagTree(Diagnostic d, String prefix, StringBuilder out) {
		diagTree(d, prefix, 0, out);
	}

	private static void diagTree(Diagnostic d, String prefix, int depth, StringBuilder out) {
		if (d == null) return;
		if (d.getSeverity() != Diagnostic.OK || depth > 0) {
			out.append(prefix).append('\t').append(depth).append('\t').append(Tape.sev(d.getSeverity())).append('\t').append(d.getCode()).append('\t')
					.append(oneLine(d.getMessage())).append('\n');
		}
		for (Diagnostic c : d.getChildren()) diagTree(c, prefix, depth + 1, out);
	}

	static String oneLine(String s) {
		return s == null ? "" : s.replace("\\", "\\\\").replace("\r", "\\r").replace("\n", "\\n").replace("\t", "\\t");
	}

	static String firstDiff(String a, String b) {
		int n = Math.min(a.length(), b.length());
		int i = 0;
		while (i < n && a.charAt(i) == b.charAt(i)) i++;
		return "first difference at char " + i + " of " + a.length() + "/" + b.length();
	}

	static void write(Path p, String s) throws IOException {
		Files.write(p, s.getBytes(StandardCharsets.UTF_8));
	}

	// --- qvto-emf's oracle classes (default package, reached by reflection) ------------------

	private static Method canonDump, suiteEqual;

	static synchronized String canonDump(List<Resource> rs) throws Exception {
		if (canonDump == null) canonDump = Class.forName("CanonDump").getMethod("dump", List.class);
		return (String) canonDump.invoke(null, rs);
	}

	static synchronized boolean suiteEqual(List<EObject> expected, List<EObject> actual) throws Exception {
		if (suiteEqual == null) suiteEqual = Class.forName("SuiteCompare").getMethod("equal", List.class, List.class);
		return (Boolean) suiteEqual.invoke(null, expected, actual);
	}

	static List<Resource> list(Resource r) {
		return Collections.singletonList(r);
	}
}
