// Dumps the QVTo-visible signatures of the Java blackbox libraries the corpus uses, as Eclipse
// QVTo 3.11 loads them (StandaloneBlackboxProvider -> JavaUnitDescriptor.load -> JavaModuleLoader,
// OperationBuilder, Java2QVTTypeResolver): the blackboxes the test bundle's and the QVTo ocl
// plugin's plugin.xml declare (javaBlackboxUnits, the legacy ocl.libraries Strings and emf.tools;
// `bundle-blackboxes`), the `blackbox`/`blackbox-modules` registrations of the case files given,
// and the units the corpus imports by Java class name (the classpath: the runner's stubs, the
// compile oracle's stubs and JDT libraries).
//
// Runs in the reference runner's environment (qvtorun.Env, the runner's classpath; package qvtorun
// for its package-private classes). Usage (see blackbox-dump.sh):
//   BlackboxDump <base dir> <case file>...
// Output (stdout), tab separated, one environment after the other:
//   env     <env>  bundle|case|classpath  <registrations: class unit module nsURIs; ...>  <cases: id,id,...|->
//   unit    <env>  <unit qualified name>  <uri>  OK|ERROR  [<load diagnostic, ' | ' separated>]
//           A library without metamodels takes its types from the packages in the compilation's
//           registry (Java2QVTTypeResolver: the registry's own keys). When an import loads it, that
//           is the case's registry before the importer's model types are resolved: empty unless
//           the case's mmfile/package directives fill it (so InvalidLibrary fails, as the suite
//           expects). `blackbox query` declarations are bound at run time with a fuller registry.
//   module  <env>  <unit>  <module>  <eClass of the module: Library|OperationalTransformation>  <java class>
//           <used metamodels: the module handle's package URIs (registration or @Module), nsURI,...>
//   op      <env>  <unit>  <module>  <name>  <kind: helper|query|mapping|constructor|operation>
//           <context type|->  <static: true|false (QVTo's isStatic annotation)>
//           <params: name:direction:type;...>  <result type>  <java: class#method(param classes)>
//           <flags: hidden (a colliding definition QVTo hides), withExecutionContext, javaStatic>
// Operations are sorted by name and Java method: Eclipse defines them in the unspecified order of
// Class.getDeclaredMethods() (it varies between runs), which decides its choice among ambiguous
// overloads ("Choosing first of ...").
// Types: OCL and QVTo library types as in data/stdlib.tsv (StdlibDump.name), metamodel classifiers
// as <nsURI>#<name>, the module itself as module:<name>.
//
// An env groups the cases whose own registrations (bundle ones aside) load to the same text, so a
// case finds its blackboxes under the env that lists it. Licence: EPL-2.0.
package qvtorun;

import java.io.*;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import org.eclipse.emf.common.util.Diagnostic;
import org.eclipse.emf.ecore.*;
import org.eclipse.m2m.internal.qvt.oml.ast.env.QvtOperationalModuleEnv;
import org.eclipse.m2m.internal.qvt.oml.ast.env.QvtOperationalStdLibrary;
import org.eclipse.m2m.internal.qvt.oml.ast.parser.HiddenElementAdapter;
import org.eclipse.m2m.internal.qvt.oml.ast.parser.QvtOperationalParserUtil;
import org.eclipse.m2m.internal.qvt.oml.blackbox.BlackboxException;
import org.eclipse.m2m.internal.qvt.oml.blackbox.BlackboxRegistry;
import org.eclipse.m2m.internal.qvt.oml.blackbox.BlackboxUnit;
import org.eclipse.m2m.internal.qvt.oml.blackbox.BlackboxUnitDescriptor;
import org.eclipse.m2m.internal.qvt.oml.blackbox.LoadContext;
import org.eclipse.m2m.internal.qvt.oml.blackbox.ResolutionContextImpl;
import org.eclipse.m2m.internal.qvt.oml.compiler.BlackboxUnitResolver;
import org.eclipse.m2m.internal.qvt.oml.expressions.Constructor;
import org.eclipse.m2m.internal.qvt.oml.expressions.Helper;
import org.eclipse.m2m.internal.qvt.oml.expressions.ImperativeOperation;
import org.eclipse.m2m.internal.qvt.oml.expressions.MappingOperation;
import org.eclipse.m2m.internal.qvt.oml.expressions.Module;
import org.eclipse.m2m.internal.qvt.oml.expressions.VarParameter;
import org.eclipse.m2m.internal.qvt.oml.stdlib.CallHandler;
import org.eclipse.m2m.internal.qvt.oml.stdlib.CallHandlerAdapter;
import org.eclipse.m2m.qvt.oml.blackbox.java.Operation;
import org.eclipse.m2m.qvt.oml.ecore.ImperativeOCL.DictionaryType;
import org.eclipse.m2m.qvt.oml.ecore.ImperativeOCL.ListType;
import org.eclipse.ocl.types.CollectionType;
import org.eclipse.ocl.types.TupleType;
import org.eclipse.ocl.types.TypeType;

public class BlackboxDump {
	static PrintStream out;

	public static void main(String[] args) throws Exception {
		out = new PrintStream(new FileOutputStream(FileDescriptor.out), true, "UTF-8");
		Path base = Paths.get(args[0]).toAbsolutePath();
		Env.init();
		BlackboxRegistry reg = BlackboxRegistry.INSTANCE;
		CaseRunner runner = new CaseRunner(base, Files.createTempDirectory("bbdump"));

		// the bundle's blackboxes, in the registry of a case without metamodel directives (a resource
		// set's registry delegating to the global one: what the compiler's LoadContext gets; with the
		// global registry itself, libraries without metamodels would infer types from every package)
		EPackage.Registry plain = runner.metamodelResolutionRS(new Job()).getPackageRegistry();
		List<Env.Bbox> bundle = Env.bundleBlackboxes();
		reg.cleanup();
		for (Env.Bbox b : bundle) Env.register(b);
		StringBuilder sb = new StringBuilder();
		dumpRegistered(plain, sb);
		out.println("env\tbundle\tbundle\t" + describe(bundle, Collections.emptyList()) + "\t-");
		out.print(sb.toString().replace("\u0000", "bundle"));

		// the cases' own registrations, in the case's metamodel registry
		Map<String, List<String>> casesByText = new LinkedHashMap<>();
		Map<String, String> regsByText = new LinkedHashMap<>();
		Set<String> imports = new TreeSet<>();
		for (int i = 1; i < args.length; i++) {
			for (Job job : Job.read(Paths.get(args[i]))) {
				imports.addAll(importsOf(base, job));
				if (job.blackboxes.isEmpty() && job.blackboxModules.isEmpty()) continue;
				reg.cleanup();
				List<Env.Bbox> own = new ArrayList<>();
				for (String[] b : job.blackboxes) {
					String[] mms = b.length > 3 ? b[3].split(",") : null;
					own.add(new Env.Bbox(b[0], b.length > 1 ? b[1] : null, b.length > 2 ? b[2] : null, mms));
				}
				for (Env.Bbox b : own) Env.register(b);
				for (String c : job.blackboxModules)
					org.eclipse.m2m.qvt.oml.TransformationExecutor.BlackboxRegistry.INSTANCE.registerModules(Class.forName(c));
				if (job.conf != null) Env.applyConf(base.resolve(job.conf), base);
				EPackage.Registry registry = runner.metamodelResolutionRS(job).getPackageRegistry();
				StringBuilder t = new StringBuilder();
				dumpRegistered(registry, t);
				String key = describe(own, job.blackboxModules) + "\n" + t;
				casesByText.computeIfAbsent(key, k -> new ArrayList<>()).add(job.id);
				regsByText.put(key, describe(own, job.blackboxModules));
			}
		}
		int n = 0;
		for (Map.Entry<String, List<String>> e : casesByText.entrySet()) {
			String env = "case" + (++n);
			out.println("env\t" + env + "\tcase\t" + regsByText.get(e.getKey()) + "\t" + String.join(",", e.getValue()));
			out.print(e.getKey().substring(e.getKey().indexOf('\n') + 1).replace("\u0000", env));
		}

		// units the corpus imports by Java class name (nothing registered)
		reg.cleanup();
		StringBuilder cp = new StringBuilder();
		List<String> found = new ArrayList<>();
		ResolutionContextImpl ctx = new ResolutionContextImpl(BlackboxUnitResolver.GLOBAL_CONTEXT);
		for (String qn : imports) {
			if (!qn.contains(".")) continue;
			BlackboxUnitDescriptor d;
			try {
				d = reg.getCompilationUnitDescriptor(qn, ctx);
			} catch (RuntimeException e) {
				d = null;
			}
			if (d == null) continue;
			found.add(qn);
			dumpUnit(d, plain, cp);
		}
		out.println("env\tclasspath\tclasspath\t" + String.join("; ", found) + "\t-");
		out.print(cp.toString().replace("\u0000", "classpath"));
		reg.cleanup();
		System.exit(0);
	}

	static String describe(List<Env.Bbox> bs, List<String> modules) {
		List<String> parts = new ArrayList<>();
		for (Env.Bbox b : bs) {
			StringBuilder s = new StringBuilder(b.cls);
			if (b.unit != null) s.append(' ').append(b.unit);
			if (b.module != null) s.append(' ').append(b.module);
			if (b.mms != null && b.mms.length > 0) s.append(' ').append(String.join(",", b.mms));
			parts.add(s.toString());
		}
		for (String m : modules) parts.add("modules " + m);
		return String.join("; ", parts);
	}

	/** Qualified names of the imports of the case's transformation (comments stripped). */
	static List<String> importsOf(Path base, Job job) {
		List<String> res = new ArrayList<>();
		try {
			Path p = base.resolve(job.path(job.qvto));
			String text = new String(Files.readAllBytes(p), StandardCharsets.UTF_8);
			text = text.replaceAll("(?s)/\\*.*?\\*/", "").replaceAll("--[^\n]*", "");
			Matcher m = Pattern.compile("(?m)^\\s*import\\s+(?:library\\s+)?([\\w.]+)\\s*;").matcher(text);
			while (m.find()) res.add(m.group(1));
		} catch (IOException | RuntimeException e) {
			// no file: no imports
		}
		return res;
	}

	/** Every unit registered in the standalone provider. env placeholder: \0. */
	static void dumpRegistered(EPackage.Registry registry, StringBuilder sb) {
		ResolutionContextImpl ctx = new ResolutionContextImpl(BlackboxUnitResolver.GLOBAL_CONTEXT);
		for (BlackboxUnitDescriptor d : new ArrayList<>(BlackboxRegistry.INSTANCE.getCompilationUnitDescriptors(ctx))) dumpUnit(d, registry, sb);
	}

	static void dumpUnit(BlackboxUnitDescriptor d, EPackage.Registry registry, StringBuilder sb) {
		String unit = d.getQualifiedName();
		BlackboxUnit u;
		try {
			u = d.load(new LoadContext(registry));
		} catch (BlackboxException e) {
			List<String> msgs = new ArrayList<>();
			flatten(e.getDiagnostic(), msgs);
			String err = String.join(" | ", msgs).replace('\t', ' ').replace('\n', ' ');
			sb.append("unit\t\u0000\t").append(unit).append('\t').append(d.getURI()).append("\tERROR\t").append(err).append('\n');
			return;
		}
		sb.append("unit\t\u0000\t").append(unit).append('\t').append(d.getURI()).append("\tOK\n");
		Map<String, Object> handles = moduleHandles(d);
		for (QvtOperationalModuleEnv env : u.getElements()) {
			Module module = env.getModuleContextType();
			List<String> mms = new ArrayList<>();
			String javaClass = moduleClass(module);
			Object h = handles.get(module.getName());
			if (h != null) {
				try {
					@SuppressWarnings("unchecked")
					List<String> used = (List<String>) call(h, "getUsedPackages");
					if (used != null) mms.addAll(used);
					javaClass = ((Class<?>) call(h, "getModuleJavaClass")).getName();
				} catch (ReflectiveOperationException e) {
					// keep what the operations say
				}
			}
			sb.append("module\t\u0000\t").append(unit).append('\t').append(module.getName()).append('\t').append(module.eClass().getName())
					.append('\t').append(javaClass).append('\t').append(String.join(",", mms)).append('\n');
			// QVTo defines them in Class.getDeclaredMethods() order, which is unspecified (it varies
			// between JVM runs): sorted by name and Java signature here
			List<String> ops = new ArrayList<>();
			for (EOperation o : module.getEOperations()) {
				StringBuilder line = new StringBuilder();
				op(unit, module, o, line);
				ops.add(line.toString());
			}
			ops.sort(Comparator.comparing((String l) -> l.split("\t")[4]).thenComparing(l -> l.split("\t")[10]));
			for (String l : ops) sb.append(l);
		}
	}

	static void flatten(Diagnostic d, List<String> out) {
		if (d == null) return;
		if (d.getMessage() != null) out.add(d.getMessage().trim());
		for (Diagnostic c : d.getChildren()) flatten(c, out);
	}

	static void op(String unit, Module module, EOperation o, StringBuilder sb) {
		String kind = o instanceof Constructor ? "constructor" : o instanceof MappingOperation ? "mapping"
				: o instanceof Helper ? (((Helper) o).isIsQuery() ? "query" : "helper") : "operation";
		String ctx = "-";
		if (o instanceof ImperativeOperation && ((ImperativeOperation) o).getContext() != null) ctx = name(((ImperativeOperation) o).getContext().getEType());
		StringBuilder ps = new StringBuilder();
		for (EParameter p : o.getEParameters()) {
			if (ps.length() > 0) ps.append(';');
			String dir = p instanceof VarParameter ? ((VarParameter) p).getKind().getLiteral() : "in";
			ps.append(p.getName()).append(':').append(dir).append(':').append(name(p.getEType()));
		}
		Method m = javaMethod(o);
		List<String> flags = new ArrayList<>();
		if (HiddenElementAdapter.isMarkedAsHidden(o)) flags.add("hidden");
		if (m != null) {
			Operation a = m.getAnnotation(Operation.class);
			if (a != null && a.withExecutionContext()) flags.add("withExecutionContext");
			if (Modifier.isStatic(m.getModifiers())) flags.add("javaStatic");
		}
		String java = "-";
		if (m != null) {
			StringBuilder j = new StringBuilder(m.getDeclaringClass().getName()).append('#').append(m.getName()).append('(');
			Class<?>[] pt = m.getParameterTypes();
			for (int i = 0; i < pt.length; i++) j.append(i > 0 ? "," : "").append(pt[i].getTypeName());
			java = j.append(')').toString();
		}
		sb.append("op\t\u0000\t").append(unit).append('\t').append(module.getName()).append('\t').append(o.getName()).append('\t').append(kind)
				.append('\t').append(ctx).append('\t').append(QvtOperationalParserUtil.isStaticOperation(o)).append('\t').append(ps)
				.append('\t').append(name(o.getEType())).append('\t').append(java).append('\t').append(String.join(",", flags)).append('\n');
	}

	/** The module handles of a Java unit descriptor (JavaUnitDescriptor.fModules), by module name. */
	static Map<String, Object> moduleHandles(BlackboxUnitDescriptor d) {
		Map<String, Object> res = new HashMap<>();
		for (Class<?> k = d.getClass(); k != null; k = k.getSuperclass()) {
			try {
				Field f = k.getDeclaredField("fModules");
				f.setAccessible(true);
				for (Object h : ((Map<?, ?>) f.get(d)).keySet()) res.put((String) call(h, "getModuleName"), h);
				break;
			} catch (NoSuchFieldException e) {
				// superclass
			} catch (ReflectiveOperationException e) {
				break;
			}
		}
		return res;
	}

	static Object call(Object o, String name) throws ReflectiveOperationException {
		for (Class<?> k = o.getClass(); k != null; k = k.getSuperclass()) {
			try {
				Method m = k.getDeclaredMethod(name);
				m.setAccessible(true);
				return m.invoke(o);
			} catch (NoSuchMethodException e) {
				// superclass
			}
		}
		throw new NoSuchMethodException(name);
	}

	/** The Java class of a loaded module: the declaring class of one of its operations' methods. */
	static String moduleClass(Module module) {
		for (EOperation o : module.getEOperations()) {
			Method m = javaMethod(o);
			if (m != null) return m.getDeclaringClass().getName();
		}
		return "-";
	}

	/** The Java method behind an operation: the handler JavaMethodHandlerFactory attached (wrapped). */
	static Method javaMethod(EOperation o) {
		CallHandler h = CallHandlerAdapter.getDispatcher(o);
		return h == null ? null : findMethod(h, 0);
	}

	static Method findMethod(Object h, int depth) {
		if (h == null || depth > 3) return null;
		for (Class<?> k = h.getClass(); k != null && k != Object.class; k = k.getSuperclass()) {
			for (Field f : k.getDeclaredFields()) {
				try {
					if (f.getType() == Method.class) {
						f.setAccessible(true);
						return (Method) f.get(h);
					}
					if (CallHandler.class.isAssignableFrom(f.getType())) {
						f.setAccessible(true);
						Method m = findMethod(f.get(h), depth + 1);
						if (m != null) return m;
					}
				} catch (ReflectiveOperationException | RuntimeException e) {
					// not accessible
				}
			}
		}
		return null;
	}

	static String name(EClassifier t) {
		if (t == null) return "null";
		if (t instanceof Module) return "module:" + t.getName();
		if (t instanceof ListType) return "List(" + name(((ListType) t).getElementType()) + ")";
		if (t instanceof DictionaryType) return "Dict(" + name(((DictionaryType) t).getKeyType()) + "," + name(((DictionaryType) t).getElementType()) + ")";
		if (t instanceof CollectionType) {
			@SuppressWarnings("unchecked")
			CollectionType<EClassifier, EOperation> c = (CollectionType<EClassifier, EOperation>) t;
			return c.getKind().getName() + "(" + name(c.getElementType()) + ")";
		}
		if (t instanceof TypeType) {
			@SuppressWarnings("unchecked")
			TypeType<EClassifier, EOperation> tt = (TypeType<EClassifier, EOperation>) t;
			return "TypeType(" + name(tt.getReferredType()) + ")";
		}
		if (t instanceof TupleType) {
			StringBuilder b = new StringBuilder("Tuple(");
			boolean first = true;
			for (EStructuralFeature p : ((EClass) t).getEStructuralFeatures()) {
				if (!first) b.append(',');
				first = false;
				b.append(p.getName()).append(':').append(name(p.getEType()));
			}
			return b.append(')').toString();
		}
		EPackage p = t.getEPackage();
		if (p != null && !isLibraryPackage(p) && p.getNsURI() != null && !p.getNsURI().isEmpty()) return p.getNsURI() + "#" + t.getName();
		String pn = p == null ? "?" : p.getName();
		return pn + "::" + t.getName();
	}

	/** The OCL standard library and QVTo's Stdlib (printed as in stdlib.tsv). */
	static boolean isLibraryPackage(EPackage p) {
		EPackage ocl = QvtOperationalStdLibrary.INSTANCE.getEnvironment().getOCLStandardLibrary().getOclAny().getEPackage();
		return p == ocl || p == QvtOperationalStdLibrary.INSTANCE.getStdLibModule();
	}
}
