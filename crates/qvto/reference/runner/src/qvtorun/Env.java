package qvtorun;

import java.io.File;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

import javax.xml.parsers.DocumentBuilderFactory;

import org.eclipse.emf.common.util.Diagnostic;
import org.eclipse.emf.ecore.EcorePackage;
import org.eclipse.emf.ecore.plugin.EcorePlugin;
import org.eclipse.emf.ecore.resource.Resource;
import org.eclipse.emf.ecore.xmi.impl.EcoreResourceFactoryImpl;
import org.eclipse.emf.ecore.xmi.impl.XMIResourceFactoryImpl;
import org.eclipse.m2m.qvt.oml.TransformationExecutor;
import org.w3c.dom.Document;
import org.w3c.dom.Element;
import org.w3c.dom.NodeList;

/**
 * The process-wide environment Eclipse provides to the test drivers.
 *
 * <ul>
 * <li>EMF's registrations from the plugin.xml files on the classpath (generated and dynamic
 * packages, platform:/plugin URI mappings), through EMF's standalone
 * {@link EcorePlugin.ExtensionProcessor}. The qvto-run script puts the test bundle and the
 * samples plugin directories on the classpath, so their plugin.xml files count, as under OSGi.</li>
 * <li>The blackbox units the test bundle's plugin.xml declares (javaBlackboxUnits, the legacy
 * ocl.libraries extension) and QVTo's legacy built-in libraries (Strings, emf.tools), for the
 * drivers that run under OSGi ({@code bundle-blackboxes} in a case).</li>
 * <li>Locale en_US, the default resource factories of a standalone EMF (ecore, *).</li>
 * </ul>
 */
final class Env {
	private Env() {}

	static boolean initialised;

	static synchronized void init() {
		if (initialised) return;
		initialised = true;
		Locale.setDefault(Locale.US);
		Resource.Factory.Registry.INSTANCE.getExtensionToFactoryMap().put("ecore", new EcoreResourceFactoryImpl());
		Resource.Factory.Registry.INSTANCE.getExtensionToFactoryMap().put("*", new XMIResourceFactoryImpl());
		EcorePackage.eINSTANCE.eClass();
		EcorePlugin.ExtensionProcessor.process(null);
		File tb = testBundle();
		if (tb != null) {
			try {
				BundleUnitResolverFactory.install(java.util.Collections.singletonList(tb.getParentFile()));
			} catch (Exception e) {
				throw new IllegalStateException(e);
			}
		}
	}

	/** Directory of the Eclipse test bundle (reference checkout). */
	static File testBundle() {
		String p = System.getProperty("qvtorun.testbundle");
		return p == null ? null : new File(p);
	}

	static File oclPlugin() {
		String p = System.getProperty("qvtorun.oclplugin");
		return p == null ? null : new File(p);
	}

	/** One blackbox registration: class, unit qualified name, module name, metamodel nsURIs. */
	static final class Bbox {
		final String cls, unit, module;
		final String[] mms;

		Bbox(String cls, String unit, String module, String[] mms) {
			this.cls = cls;
			this.unit = unit;
			this.module = module;
			this.mms = mms;
		}
	}

	static List<Bbox> bundleBlackboxes;

	/** The blackboxes of the test bundle's and the QVTo ocl plugin's plugin.xml. */
	static synchronized List<Bbox> bundleBlackboxes() throws Exception {
		if (bundleBlackboxes != null) return bundleBlackboxes;
		List<Bbox> out = new ArrayList<>();
		for (File dir : new File[] { oclPlugin(), testBundle() }) {
			if (dir == null) continue;
			File px = new File(dir, "plugin.xml");
			if (!px.isFile()) continue;
			Document d = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(px);
			NodeList exts = d.getElementsByTagName("extension");
			for (int i = 0; i < exts.getLength(); i++) {
				Element ext = (Element) exts.item(i);
				String point = ext.getAttribute("point");
				if (point.equals("org.eclipse.m2m.qvt.oml.javaBlackboxUnits")) {
					for (Element e : children(ext)) {
						if (e.getTagName().equals("unit")) {
							String unit = e.getAttribute("namespace") + "." + e.getAttribute("name");
							for (Element lib : children(e)) if (lib.getTagName().equals("library")) out.add(library(lib, unit));
						} else if (e.getTagName().equals("library")) {
							out.add(library(e, null));
						}
					}
				} else if (point.equals("org.eclipse.m2m.qvt.oml.ocl.libraries")) {
					for (Element lib : children(ext)) {
						if (!lib.getTagName().equals("library")) continue;
						List<String> mms = new ArrayList<>();
						for (Element m : children(lib)) {
							String u = m.getAttribute("uri");
							if (!u.isEmpty() && !mms.contains(u)) mms.add(u);
						}
						String id = lib.getAttribute("id");
						out.add(new Bbox(lib.getAttribute("class"), id, id, mms.toArray(new String[0])));
					}
				}
			}
		}
		bundleBlackboxes = out;
		return out;
	}

	private static Bbox library(Element lib, String unit) {
		List<String> mms = new ArrayList<>();
		for (Element m : children(lib)) if (m.getTagName().equals("metamodel")) mms.add(m.getAttribute("nsURI"));
		String name = lib.getAttribute("name");
		return new Bbox(lib.getAttribute("class"), unit, name.isEmpty() ? null : name, mms.toArray(new String[0]));
	}

	private static List<Element> children(Element e) {
		List<Element> out = new ArrayList<>();
		NodeList l = e.getChildNodes();
		for (int i = 0; i < l.getLength(); i++) if (l.item(i) instanceof Element) out.add((Element) l.item(i));
		return out;
	}

	static Diagnostic register(Bbox b) throws ClassNotFoundException {
		Class<?> c = Class.forName(b.cls);
		TransformationExecutor.BlackboxRegistry reg = TransformationExecutor.BlackboxRegistry.INSTANCE;
		if (b.unit == null) return reg.registerModule(c);
		if (b.mms == null || b.mms.length == 0) return reg.registerModule(c, b.unit, b.module);
		return reg.registerModule(c, b.unit, b.module, b.mms);
	}
}
