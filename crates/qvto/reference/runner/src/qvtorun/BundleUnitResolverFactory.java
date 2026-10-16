package qvtorun;

import java.io.File;
import java.lang.reflect.Field;
import java.util.ArrayList;
import java.util.List;

import javax.xml.parsers.DocumentBuilderFactory;

import org.eclipse.emf.common.util.URI;
import org.eclipse.m2m.internal.qvt.oml.compiler.ResolverUtils;
import org.eclipse.m2m.internal.qvt.oml.compiler.URIUnitResolver;
import org.eclipse.m2m.internal.qvt.oml.compiler.UnitResolver;
import org.eclipse.m2m.internal.qvt.oml.compiler.UnitResolverFactory;
import org.w3c.dom.Document;
import org.w3c.dom.Element;
import org.w3c.dom.NodeList;

/**
 * Import resolution as under OSGi for transformations inside a bundle directory: units resolve
 * against the bundle's source containers (the {@code qvtTransformationContainer} extensions of
 * its plugin.xml; the bundle root if it declares none), as QVTo's PlatformPluginUnitResolver
 * does for {@code platform:/plugin} URIs. The qualified name of a unit is its path relative to
 * the first container that contains it.
 *
 * <p>Inserted before QVTo's default {@code URIUnitResolverFactory} (which resolves imports
 * relative to the importing file's folder). Cases that compile like TestQvtParser (imports
 * relative to the case folder) switch it off with {@link #enabled}.
 */
final class BundleUnitResolverFactory extends UnitResolverFactory {
	static final class Bundle {
		final File root;
		final List<String> containers;

		Bundle(File root, List<String> containers) {
			this.root = root;
			this.containers = containers;
		}
	}

	static final List<Bundle> bundles = new ArrayList<>();
	static volatile boolean enabled = true;
	/** A source root for the current case (imports root DIR), instead of the bundles. */
	static volatile Bundle caseRoot;

	/** Registers the bundle directories (with a plugin.xml) below the given directories. */
	static void install(List<File> bundleParents) throws Exception {
		for (File parent : bundleParents) {
			File[] dirs = parent.listFiles(File::isDirectory);
			if (dirs == null) continue;
			java.util.Arrays.sort(dirs);
			for (File d : dirs) {
				File px = new File(d, "plugin.xml");
				if (!px.isFile()) continue;
				List<String> containers = new ArrayList<>();
				Document doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(px);
				NodeList l = doc.getElementsByTagName("sourceContainer");
				for (int i = 0; i < l.getLength(); i++) {
					String p = ((Element) l.item(i)).getAttribute("path").trim();
					if (p.isEmpty()) continue;
					containers.add(p.equals("/") ? "" : p.replaceAll("^/+|/+$", ""));
				}
				if (containers.isEmpty()) containers.add("");
				bundles.add(new Bundle(d.getCanonicalFile(), containers));
			}
		}
		Object registry = UnitResolverFactory.Registry.INSTANCE;
		Field f = registry.getClass().getDeclaredField("fFactories");
		f.setAccessible(true);
		@SuppressWarnings("unchecked")
		List<UnitResolverFactory> factories = (List<UnitResolverFactory>) f.get(registry);
		factories.add(0, new BundleUnitResolverFactory());
	}

	private static String canonical(URI uri) {
		try {
			return new File(uri.toFileString()).getCanonicalPath();
		} catch (java.io.IOException e) {
			return new File(uri.toFileString()).getAbsolutePath();
		}
	}

	private static Bundle bundleOf(URI uri) {
		if (!uri.isFile()) return null;
		String path = canonical(uri);
		if (caseRoot != null) return path.startsWith(caseRoot.root.getPath() + File.separator) ? caseRoot : null;
		for (Bundle b : bundles) if (path.startsWith(b.root.getPath() + File.separator)) return b;
		return null;
	}

	@Override
	protected boolean accepts(URI uri) {
		return enabled && bundleOf(uri) != null && getQualifiedName(uri) != null;
	}

	@Override
	protected UnitResolver getResolver(URI uri) {
		Bundle b = bundleOf(uri);
		// the bundle root as the URI spells it (not canonical: unit URIs keep the caller's path)
		String abs = new File(uri.toFileString()).getAbsolutePath();
		String rel = canonical(uri).substring(b.root.getPath().length());
		File root = abs.endsWith(rel) ? new File(abs.substring(0, abs.length() - rel.length())) : b.root;
		List<URI> bases = new ArrayList<>();
		for (String c : b.containers) {
			File dir = c.isEmpty() ? root : new File(root, c);
			bases.add(URI.createFileURI(dir.getPath() + File.separator));
		}
		return new URIUnitResolver(bases);
	}

	@Override
	protected String getQualifiedName(URI uri) {
		Bundle b = bundleOf(uri);
		if (b == null) return null;
		String rel = canonical(uri).substring(b.root.getPath().length() + 1);
		int dot = rel.lastIndexOf('.');
		if (dot > rel.lastIndexOf(File.separatorChar)) rel = rel.substring(0, dot);
		// strip the first container with a matching first segment (PlatformPluginUnitResolverFactory)
		for (String c : b.containers) {
			if (c.isEmpty()) continue;
			if (rel.startsWith(c + File.separator)) {
				rel = rel.substring(c.length() + 1);
				break;
			}
		}
		return ResolverUtils.toQualifiedName(rel.replace(File.separatorChar, '/'));
	}
}
