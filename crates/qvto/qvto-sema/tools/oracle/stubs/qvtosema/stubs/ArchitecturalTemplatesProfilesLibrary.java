package qvtosema.stubs;

import java.util.Collection;
import java.util.List;
import java.util.Set;

import org.eclipse.emf.ecore.EObject;

/**
 * Stub of Palladio's org.palladiosimulator.architecturaltemplates.catalog.black.ProfilesLibrary
 * (architecturaltemplates.catalog 5.2.2, EPL-2.0), registered under that unit name by the
 * compile-oracle cases: every public operation of the original (no annotations: plain static
 * helpers), with the generated PCM types (Entity, NamedElement) and EMF Profiles' Stereotype
 * replaced by EObject. Unlike reference/runner/src-stubs (only what the SPD case needs), it has
 * the Set overloads the SPD helpers call. Every operation fails if called.
 */
public class ArchitecturalTemplatesProfilesLibrary {
	public ArchitecturalTemplatesProfilesLibrary() {
	}

	private static RuntimeException stub() {
		return new UnsupportedOperationException("qvto-rs compile oracle: ProfilesLibrary stub");
	}

	public static boolean isProfileApplied(EObject eObject, String profileName) {
		throw stub();
	}

	public static void applyProfile(EObject eObject, String profileName) {
		throw stub();
	}

	public static boolean hasAppliedStereotype(EObject entity, String stereotypeName) {
		throw stub();
	}

	public static boolean hasAppliedStereotype(Set<EObject> entities, String stereotypeName) {
		throw stub();
	}

	public static Collection<EObject> getAppliedStereotypes(EObject entity) {
		throw stub();
	}

	public static void applyStereotype(EObject entity, String stereotypeName) {
		throw stub();
	}

	public static void removeStereotypeApplications(EObject entity, String stereotypeName) {
		throw stub();
	}

	public static void setTaggedValue(EObject entity, Object value, String taggedValueName, String stereotypeName) {
		throw stub();
	}

	public static void setTaggedValue(EObject entity, int value, String taggedValueName, String stereotypeName) {
		throw stub();
	}

	public static void setStringTaggedValue(EObject entity, String value, String taggedValueName, String stereotypeName) {
		throw stub();
	}

	public static void setDoubleTaggedValue(EObject entity, double value, String taggedValueName, String stereotypeName) {
		throw stub();
	}

	public static Object getTaggedValue(EObject entity, String taggedValueName, String stereotypeName) {
		throw stub();
	}

	public static int getIntTaggedValue(EObject entity, String taggedValueName, String stereotypeName) {
		throw stub();
	}

	public static long getLongTaggedValue(EObject entity, String taggedValueName, String stereotypeName) {
		throw stub();
	}

	public static double getDoubleTaggedValue(EObject entity, String taggedValueName, String stereotypeName) {
		throw stub();
	}

	public static String getStringTaggedValue(EObject entity, String taggedValueName, String stereotypeName) {
		throw stub();
	}

	public static boolean appliedStereotypesEqualsOne(Set<EObject> entities, String stereotypeName) {
		throw stub();
	}

	public static void delete(List<EObject> rootEObjects, EObject eObject) {
		throw stub();
	}
}
