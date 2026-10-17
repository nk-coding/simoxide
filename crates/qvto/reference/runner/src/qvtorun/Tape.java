package qvtorun;

import java.io.IOException;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.IdentityHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

import org.eclipse.emf.common.util.Diagnostic;
import org.eclipse.emf.common.util.TreeIterator;
import org.eclipse.emf.ecore.EObject;
import org.eclipse.emf.ecore.EPackage;
import org.eclipse.emf.ecore.resource.Resource;
import org.eclipse.emf.ecore.util.EcoreUtil;

/**
 * Identity-hash tape: the {@code System.identityHashCode} values HotSpot assigned during a run,
 * so that qvto-rs's compat mode can replay the iteration order of identity-hashed sets and maps
 * (DESIGN.md 6.1).
 *
 * <p>HotSpot stores an object's identity hash in its mark word the first time it is requested
 * (default {@code -XX:hashCode=5}: a per-thread Marsaglia xor-shift sequence seeded at thread
 * creation from {@code os::random()}). After the run the tape reads each object's mark word
 * (through {@code sun.misc.Unsafe}) before asking for its hash, so it can tell the hashes the run
 * assigned from those it did not request. Lines:
 *
 * <pre>
 * seed  &lt;h1&gt;  &lt;h2&gt;            first two hashes of two fresh objects on the case's fresh thread;
 *                              they determine the thread's xor-shift state (see the testkit)
 * obj   &lt;key&gt;  &lt;hash&gt;  &lt;r&gt;   r = 1: assigned during the run, 0: assigned by the tape itself,
 *                              ? : mark word unreadable (locked or unsupported VM)
 * </pre>
 *
 * Keys: {@code <resource label>#<canonical path>} (CanonDump) for objects in the input
 * resources and the output extents, {@code mm:<nsURI>#<fragment>} for the objects of the
 * metamodels these objects are instances of. Objects that the run created but that end up in no
 * output extent (intermediate data, discarded objects, Status, exceptions, tuples) are not on the
 * tape.
 *
 * <p>The objects of the input resources come first, as {@code pre} lines keyed by their path
 * before the run ({@link #pre}): a run that reorders an inout model moves them, and the engine
 * replaying the tape knows only where they were. The {@code obj} lines are the other objects.
 */
final class Tape {
	private Tape() {}

	private static Object unsafe;
	private static Method getLong;

	static synchronized long mark(Object o) {
		try {
			if (getLong == null) {
				Class<?> c = Class.forName("sun.misc.Unsafe");
				Field f = c.getDeclaredField("theUnsafe");
				f.setAccessible(true);
				unsafe = f.get(null);
				getLong = c.getMethod("getLong", Object.class, long.class);
			}
			return (Long) getLong.invoke(unsafe, o, 0L);
		} catch (Exception e) {
			return -1;
		}
	}

	/** "1" if the identity hash was assigned before (unlocked mark word with hash bits), "0" if not, "?" else. */
	static String assigned(Object o) {
		long m = mark(o);
		if (m == -1 || (m & 3) != 1) return "?";
		return ((m >>> 8) & 0x7FFFFFFFL) != 0 ? "1" : "0";
	}

	static int[] seed() {
		return new int[] { System.identityHashCode(new Object()), System.identityHashCode(new Object()) };
	}

	/** The objects of the input resources and their keys before the run (no identity hash is requested). */
	static final class Pre {
		final List<EObject> objs = new ArrayList<>();
		final List<String> keys = new ArrayList<>();
	}

	static Pre pre(List<Resource> inputs) {
		Pre p = new Pre();
		for (Resource r : inputs) {
			String l = label(r);
			for (EObject root : r.getContents()) {
				p.objs.add(root);
				p.keys.add(l + "#" + path(root));
				for (TreeIterator<EObject> it = root.eAllContents(); it.hasNext();) {
					EObject o = it.next();
					p.objs.add(o);
					p.keys.add(l + "#" + path(o));
				}
			}
		}
		return p;
	}

	/** CanonDump.label, without reflection (no identity hash may be requested before the run). */
	private static String label(Resource r) {
		org.eclipse.emf.common.util.URI u = r.getURI();
		if (u.isFile() && u.lastSegment() != null) return org.eclipse.emf.common.util.URI.decode(u.lastSegment());
		return u.toString();
	}

	/** CanonDump.path, likewise. */
	private static String path(EObject o) {
		List<String> segs = new ArrayList<>();
		EObject cur = o;
		while (cur.eContainer() != null && ((org.eclipse.emf.ecore.InternalEObject) cur).eDirectResource() == null) {
			org.eclipse.emf.ecore.EReference f = cur.eContainmentFeature();
			EObject p = cur.eContainer();
			if (f.isMany()) segs.add("@" + f.getName() + "." + ((List<?>) p.eGet(f, false)).indexOf(cur));
			else segs.add("@" + f.getName());
			cur = p;
		}
		Resource r = ((org.eclipse.emf.ecore.InternalEObject) cur).eDirectResource();
		StringBuilder sb = new StringBuilder("/");
		sb.append(r == null ? "" : String.valueOf(r.getContents().indexOf(cur)));
		for (int i = segs.size() - 1; i >= 0; i--) sb.append('/').append(segs.get(i));
		return sb.toString();
	}

	/** Objects of a run and whether their identity hash was assigned, read right after the run. */
	static final class Snapshot {
		final List<EObject> objs = new ArrayList<>();
		final List<String> assigned = new ArrayList<>();
		final List<EPackage> packages = new ArrayList<>();

		final Map<Object, Boolean> index = new IdentityHashMap<>();
		int firstMeta;
		/** The first {@code nPre} objects are those of {@link Pre}, with these keys. */
		int nPre;
		List<String> preKeys = List.of();
	}

	static Snapshot snapshot(Pre pre, List<Resource> inputs, List<? extends List<EObject>> outputs) {
		Snapshot s = new Snapshot();
		if (pre != null) {
			// in the order of the walk before the run (which the snapshot's walk repeats when the
			// run moves nothing); mark words read before any hash is requested here
			for (EObject o : pre.objs) {
				s.objs.add(o);
				s.assigned.add(assigned(o));
			}
			for (EObject o : pre.objs) s.index.put(o, Boolean.TRUE);
			s.nPre = pre.objs.size();
			s.preKeys = pre.keys;
			for (EObject o : pre.objs) {
				EPackage p = o.eClass().getEPackage();
				if (p != null) {
					boolean known = false;
					for (EPackage q : s.packages) known |= q == p;
					if (!known) s.packages.add(p);
				}
			}
		}
		List<EObject> roots = new ArrayList<>();
		for (Resource r : inputs) roots.addAll(r.getContents());
		for (List<EObject> l : outputs) roots.addAll(l);
		for (EObject root : roots) {
			add(s, root);
			for (TreeIterator<EObject> it = root.eAllContents(); it.hasNext();) add(s, it.next());
		}
		s.firstMeta = s.objs.size();
		for (int i = 0; i < s.packages.size(); i++) {
			for (TreeIterator<EObject> it = s.packages.get(i).eAllContents(); it.hasNext();) add(s, it.next());
			add(s, s.packages.get(i));
		}
		return s;
	}

	private static void add(Snapshot s, EObject o) {
		String a = assigned(o);
		if (s.index.containsKey(o)) return;
		s.index.put(o, Boolean.TRUE);
		s.objs.add(o);
		s.assigned.add(a);
		EPackage p = o.eClass().getEPackage();
		if (p != null) {
			boolean known = false;
			for (EPackage q : s.packages) known |= q == p;
			if (!known) s.packages.add(p);
		}
	}

	static void write(Path file, int[] seed, Snapshot s) throws Exception {
		Method path = Class.forName("CanonDump").getDeclaredMethod("path", EObject.class);
		Method label = Class.forName("CanonDump").getDeclaredMethod("label", Resource.class);
		path.setAccessible(true);
		label.setAccessible(true);
		StringBuilder sb = new StringBuilder();
		sb.append("seed\t").append(seed[0]).append('\t').append(seed[1]).append('\n');
		for (int i = 0; i < s.objs.size(); i++) {
			EObject o = s.objs.get(i);
			String key;
			if (i < s.nPre) {
				sb.append("pre\t").append(s.preKeys.get(i)).append('\t').append(System.identityHashCode(o)).append('\t').append(s.assigned.get(i)).append('\n');
				continue;
			}
			if (i >= s.firstMeta) {
				EPackage p = packageOf(o);
				String frag = o.eResource() != null ? o.eResource().getURIFragment(o) : EcoreUtil.getURI(o).fragment();
				key = "mm:" + (p == null ? "" : p.getNsURI()) + "#" + frag;
			} else if (o.eResource() != null) {
				key = label.invoke(null, o.eResource()) + "#" + path.invoke(null, o);
			} else {
				key = "!" + path.invoke(null, o);
			}
			sb.append("obj\t").append(key).append('\t').append(System.identityHashCode(o)).append('\t').append(s.assigned.get(i)).append('\n');
		}
		Files.write(file, sb.toString().getBytes(StandardCharsets.UTF_8));
	}

	private static EPackage packageOf(EObject o) {
		for (EObject c = o; c != null; c = c.eContainer()) if (c instanceof EPackage && c.eContainer() == null) return (EPackage) c;
		return null;
	}

	static String sev(int s) {
		switch (s) {
		case Diagnostic.OK: return "OK";
		case Diagnostic.INFO: return "INFO";
		case Diagnostic.WARNING: return "WARNING";
		case Diagnostic.ERROR: return "ERROR";
		case Diagnostic.CANCEL: return "CANCEL";
		default: return String.valueOf(s);
		}
	}

	static String fnv(byte[] b) {
		long h = 0xcbf29ce484222325L;
		for (byte x : b) {
			h ^= (x & 0xff);
			h *= 0x100000001b3L;
		}
		return String.format("%016x", h);
	}
}
