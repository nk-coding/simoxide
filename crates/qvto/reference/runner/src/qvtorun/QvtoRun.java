package qvtorun;

import java.io.File;
import java.io.PrintWriter;
import java.io.StringWriter;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import org.eclipse.emf.common.util.Diagnostic;
import org.eclipse.emf.common.util.TreeIterator;
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
import org.eclipse.m2m.qvt.oml.BasicModelExtent;
import org.eclipse.m2m.qvt.oml.ExecutionContextImpl;
import org.eclipse.m2m.qvt.oml.ExecutionDiagnostic;
import org.eclipse.m2m.qvt.oml.ModelExtent;
import org.eclipse.m2m.qvt.oml.TransformationExecutor;
import org.eclipse.m2m.qvt.oml.util.WriterLog;

/**
 * Headless Eclipse QVTo runner (reference oracle for qvto-rs).
 *
 * Usage:
 *   qvto-run --qvto T.qvto [--mm a.ecore,...] [--register-ecore-files]
 *            --param in:a.xmi --param out:b.xmi --param inout:c.xmi
 *            [--config key=value]... [--log log.txt] [--repeat N] [--blackbox fq.Class]...
 *            [--dump out.jsonl] [--expect expected.xmi]
 *   qvto-run --batch CASES --out DIR ...   (see Batch)
 *
 * Model parameters are positional, in the order of the transformation signature.
 * "in" and "inout" load a file; "out" starts empty; "out" and "inout" are saved after the run
 * ("inout" to its path + ".out" unless --inout-in-place). The output format is XMI 2.0 with
 * the same save options for every run, so outputs can be compared textually and canonically.
 */
public final class QvtoRun {

	public static void main(String[] args) throws Exception {
		String qvto = null;
		List<String> mms = new ArrayList<>();
		List<String[]> params = new ArrayList<>();
		Map<String, String> config = new LinkedHashMap<>();
		List<String> blackboxes = new ArrayList<>();
		String logPath = null;
		int repeat = 1;
		boolean inoutInPlace = false;
		String expect = null;
		String dumpPath = null;
		for (int i = 0; i < args.length; i++) {
			switch (args[i]) {
			case "--qvto": qvto = args[++i]; break;
			case "--mm": for (String s : args[++i].split(",")) if (!s.isEmpty()) mms.add(s); break;
			case "--param": {
				String p = args[++i];
				int c = p.indexOf(':');
				params.add(new String[] { p.substring(0, c), p.substring(c + 1) });
				break;
			}
			case "--config": {
				String p = args[++i];
				int c = p.indexOf('=');
				config.put(p.substring(0, c), p.substring(c + 1));
				break;
			}
			case "--blackbox": blackboxes.add(args[++i]); break;
			case "--log": logPath = args[++i]; break;
			case "--repeat": repeat = Integer.parseInt(args[++i]); break;
			case "--inout-in-place": inoutInPlace = true; break;
			case "--expect": expect = args[++i]; break;
			case "--dump": dumpPath = args[++i]; break;
			default: throw new IllegalArgumentException("unknown argument " + args[i]);
			}
		}
		if (qvto == null) throw new IllegalArgumentException("--qvto is required");

		// the same environment as the batch mode (plugin.xml registrations, bundle source containers)
		Env.init();

		// Metamodels: load and register every (sub)package by nsURI.
		ResourceSet mmSet = new ResourceSetImpl();
		for (String mm : mms) {
			Resource r = mmSet.getResource(URI.createFileURI(new File(mm).getAbsolutePath()), true);
			for (EObject o : r.getContents()) if (o instanceof EPackage) register((EPackage) o);
		}

		for (String bb : blackboxes) {
			Diagnostic d = TransformationExecutor.BlackboxRegistry.INSTANCE.registerModules(Class.forName(bb));
			if (d.getSeverity() > Diagnostic.WARNING) System.err.println("blackbox " + bb + ": " + d);
		}

		URI tUri = URI.createFileURI(new File(qvto).getAbsolutePath());
		long t0 = System.nanoTime();
		TransformationExecutor executor = new TransformationExecutor(tUri);
		Diagnostic load = executor.loadTransformation();
		long tLoad = System.nanoTime() - t0;
		System.out.println("load: " + sev(load.getSeverity()) + " " + String.format("%.1f ms", tLoad / 1e6));
		printDiag(load, "  ");
		if (load.getSeverity() > Diagnostic.WARNING) System.exit(2);

		ExecutionDiagnostic result = null;
		List<ModelExtent> extents = null;
		List<Resource> inputs = null;
		StringWriter logBuf = null;
		for (int run = 0; run < repeat; run++) {
			ResourceSet rs = new ResourceSetImpl();
			extents = new ArrayList<>();
			inputs = new ArrayList<>();
			for (String[] p : params) {
				if (p[0].equals("out")) {
					extents.add(new BasicModelExtent());
					inputs.add(null);
				} else {
					Resource r = rs.getResource(URI.createFileURI(new File(p[1]).getAbsolutePath()), true);
					extents.add(new BasicModelExtent(r.getContents()));
					inputs.add(r);
				}
			}
			ExecutionContextImpl ctx = new ExecutionContextImpl();
			for (Map.Entry<String, String> e : config.entrySet()) ctx.setConfigProperty(e.getKey(), e.getValue());
			logBuf = new StringWriter();
			ctx.setLog(new WriterLog(logBuf));
			long t1 = System.nanoTime();
			result = executor.execute(ctx, extents.toArray(new ModelExtent[0]));
			long dt = System.nanoTime() - t1;
			System.out.println("run " + run + ": " + sev(result.getSeverity()) + " code=" + result.getCode()
					+ String.format(" %.3f ms", dt / 1e6));
		}
		printDiag(result, "  ");
		if (result.getStackTrace() != null && !result.getStackTrace().isEmpty()) {
			System.out.println("stack:");
			StringWriter sw = new StringWriter();
			result.printStackTrace(new PrintWriter(sw));
			System.out.print(sw);
		}
		String log = logBuf.toString();
		if (logPath != null) {
			try (PrintWriter w = new PrintWriter(logPath, "UTF-8")) { w.print(log); }
		} else if (!log.isEmpty()) {
			System.out.println("log:");
			System.out.print(log);
		}

		// Save outputs.
		Map<Object, Object> opts = new HashMap<>();
		opts.put(XMLResource.OPTION_ENCODING, "UTF-8");
		opts.put(XMLResource.OPTION_SCHEMA_LOCATION, Boolean.TRUE);
		ResourceSet outSet = new ResourceSetImpl();
		List<Resource> saved = new ArrayList<>();
		for (int i = 0; i < params.size(); i++) {
			String[] p = params.get(i);
			if (p[0].equals("in")) continue;
			String path = p[0].equals("inout") && !inoutInPlace ? p[1] + ".out" : p[1];
			Resource out = outSet.createResource(URI.createFileURI(new File(path).getAbsolutePath()));
			out.getContents().addAll(extents.get(i).getContents());
			out.save(opts);
			saved.add(out);
			System.out.println("saved " + p[0] + " -> " + path + " (" + count(out) + " objects)");
		}
		if (dumpPath != null) {
			// canonical dump (simoxide-emf/oracle/src/CanonDump.java) of the saved out/inout models
			String dump = CaseRunner.canonDump(saved);
			try (PrintWriter w = new PrintWriter(dumpPath, "UTF-8")) { w.print(dump); }
		}
		if (expect != null) {
			// Order-sensitive structural comparison (EcoreUtil.equals) of the first out/inout
			// extent with the expected file. A quick check only; the harness compares canonically.
			Resource exp = new ResourceSetImpl().getResource(URI.createFileURI(new File(expect).getAbsolutePath()), true);
			List<EObject> actual = null;
			for (int i = 0; i < params.size(); i++) if (!params.get(i)[0].equals("in")) { actual = extents.get(i).getContents(); break; }
			boolean eq = actual != null && org.eclipse.emf.ecore.util.EcoreUtil.equals(actual, exp.getContents());
			System.out.println("expect: " + (eq ? "EQUAL" : "DIFFERENT"));
			if (!eq) System.exit(3);
		}
		System.exit(result.getSeverity() > Diagnostic.WARNING ? 1 : 0);
	}

	private static void register(EPackage p) {
		if (p.getNsURI() != null) EPackage.Registry.INSTANCE.put(p.getNsURI(), p);
		for (EPackage s : p.getESubpackages()) register(s);
	}

	private static int count(Resource r) {
		int n = 0;
		for (TreeIterator<EObject> it = r.getAllContents(); it.hasNext(); it.next()) n++;
		return n;
	}

	private static String sev(int s) {
		switch (s) {
		case Diagnostic.OK: return "OK";
		case Diagnostic.INFO: return "INFO";
		case Diagnostic.WARNING: return "WARNING";
		case Diagnostic.ERROR: return "ERROR";
		case Diagnostic.CANCEL: return "CANCEL";
		default: return String.valueOf(s);
		}
	}

	private static void printDiag(Diagnostic d, String indent) {
		if (d.getSeverity() != Diagnostic.OK && d.getMessage() != null) System.out.println(indent + sev(d.getSeverity()) + ": " + d.getMessage());
		List<Diagnostic> ch = d.getChildren() == null ? Collections.emptyList() : d.getChildren();
		for (Diagnostic c : ch) printDiag(c, indent + "  ");
	}
}
