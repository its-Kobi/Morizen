package dev.morizen.compat.quilt.provider;

import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Collection;
import java.util.List;
import org.quiltmc.loader.api.Version;
import org.quiltmc.loader.impl.entrypoint.GameTransformer;
import org.quiltmc.loader.impl.game.EmptyMappingConfiguration;
import org.quiltmc.loader.impl.game.GameProvider;
import org.quiltmc.loader.impl.game.MappingConfiguration;
import org.quiltmc.loader.impl.launch.common.QuiltLauncher;
import org.quiltmc.loader.impl.launch.knot.KnotClient;
import org.quiltmc.loader.impl.metadata.qmj.V1ModMetadataBuilder;
import org.quiltmc.loader.impl.util.Arguments;

/** Quilt Loader 0.30.2 provider for the native Morizen engine. */
public final class MorizenQuiltGameProvider implements GameProvider {
    private static final String BOOTSTRAP_CLASS = "dev.morizen.compat.quilt.QuiltBootstrap";
    private static volatile ClassLoader activeTargetLoader;

    private final Arguments arguments = new Arguments();
    private final GameTransformer transformer = new GameTransformer();
    private final MappingConfiguration mappings = new EmptyMappingConfiguration("official");
    private Path launchDirectory;
    private Path bootstrapJar;

    public static void launchKnot(String[] arguments) {
        try {
            KnotClient.main(arguments);
        } catch (Error fatal) {
            throw fatal;
        } catch (Throwable failure) {
            throw new IllegalStateException(formatFailure(failure), failure);
        }
    }

    @Override
    public String getGameId() {
        return "minecraft";
    }

    @Override
    public String getGameName() {
        return "Morizen";
    }

    @Override
    public String getRawGameVersion() {
        return "26.3";
    }

    @Override
    public String getNormalizedGameVersion() {
        return "26.3";
    }

    @Override
    public Collection<BuiltinMod> getBuiltinMods() {
        List<Path> paths = List.of(bootstrapJar);
        return List.of(
                new BuiltinMod(paths, metadata("minecraft", "net.minecraft", "26.3", "Minecraft")),
                new BuiltinMod(paths, metadata("morizen", "dev.morizen", "0.1.0", "Morizen")));
    }

    private static org.quiltmc.loader.impl.metadata.qmj.InternalModMetadata metadata(
            String id, String group, String version, String name) {
        V1ModMetadataBuilder builder = new V1ModMetadataBuilder();
        builder.setId(id);
        builder.setGroup(group);
        builder.setVersion(Version.of(version));
        builder.setName(name);
        builder.setDescription(name + " built in to the native Morizen host");
        builder.setIntermediateMappings("official");
        return builder.build();
    }

    @Override
    public String getEntrypoint() {
        return BOOTSTRAP_CLASS;
    }

    @Override
    public Path getLaunchDirectory() {
        return launchDirectory;
    }

    @Override
    public MappingConfiguration getMappingConfiguration() {
        return mappings;
    }

    @Override
    public boolean requiresUrlClassLoader() {
        return false;
    }

    @Override
    public boolean isEnabled() {
        return true;
    }

    @Override
    public boolean locateGame(QuiltLauncher launcher, String[] launchArguments) {
        arguments.parse(launchArguments);
        launchDirectory = requiredPath("morizen.gameDir");
        bootstrapJar = requiredPath("morizen.bootstrapJar");
        if (!Files.isDirectory(launchDirectory) || !Files.isRegularFile(bootstrapJar)) {
            throw new IllegalStateException("Morizen Quilt bridge paths do not exist");
        }
        return true;
    }

    @Override
    public void initialize(QuiltLauncher launcher) {
        transformer.locateEntrypoints(launcher, List.of(bootstrapJar));
    }

    @Override
    public GameTransformer getEntrypointTransformer() {
        return transformer;
    }

    @Override
    public void unlockClassPath(QuiltLauncher launcher) {
        launcher.addToClassPath(bootstrapJar);
        String shims = System.getProperty("morizen.shimsJar");
        if (shims != null && !shims.isBlank()) {
            Path shimJar = Path.of(shims).toAbsolutePath().normalize();
            if (!Files.isRegularFile(shimJar)) {
                throw new IllegalStateException("Morizen shim JAR does not exist: " + shimJar);
            }
            launcher.addToClassPath(shimJar);
        }
    }

    @Override
    public void launch(ClassLoader targetLoader) {
        activeTargetLoader = targetLoader;
        System.out.println("morizen-java: quilt-provider=" + getClass().getName());
        invokeBootstrap(targetLoader, "start", new Class<?>[] {Path.class}, launchDirectory);
    }

    @Override
    public List<Path> getGameJars(String namespace) {
        return List.of(bootstrapJar);
    }

    public static ClassLoader getActiveTargetLoader() {
        if (activeTargetLoader == null) {
            throw new IllegalStateException("Quilt target class loader is not active");
        }
        return activeTargetLoader;
    }

    public static String dispatchActive(String wire) {
        Object value = invokeBootstrapResult(
                getActiveTargetLoader(), "dispatch", new Class<?>[] {String.class}, wire);
        return (String) value;
    }

    public static boolean hasCallbacksActive() {
        Object value = invokeBootstrapResult(
                getActiveTargetLoader(), "hasCallbacks", new Class<?>[0]);
        return Boolean.TRUE.equals(value);
    }

    public static void stopActive() {
        ClassLoader loader = activeTargetLoader;
        if (loader == null) {
            return;
        }
        try {
            invokeBootstrap(loader, "stop", new Class<?>[0]);
        } finally {
            activeTargetLoader = null;
        }
    }

    private static Path requiredPath(String property) {
        String value = System.getProperty(property);
        if (value == null || value.isBlank()) {
            throw new IllegalStateException("Missing required system property " + property);
        }
        return Path.of(value).toAbsolutePath().normalize();
    }

    private static void invokeBootstrap(
            ClassLoader loader, String method, Class<?>[] parameterTypes, Object... arguments) {
        invokeBootstrapResult(loader, method, parameterTypes, arguments);
    }

    private static Object invokeBootstrapResult(
            ClassLoader loader, String method, Class<?>[] parameterTypes, Object... arguments) {
        try {
            Class<?> bootstrap = Class.forName(BOOTSTRAP_CLASS, true, loader);
            Method target = bootstrap.getMethod(method, parameterTypes);
            return target.invoke(null, arguments);
        } catch (InvocationTargetException error) {
            Throwable cause = error.getCause();
            if (cause instanceof Error fatal) {
                throw fatal;
            }
            throw new IllegalStateException(formatFailure(cause), cause);
        } catch (ReflectiveOperationException error) {
            throw new IllegalStateException("Could not invoke Morizen Quilt bootstrap " + method, error);
        }
    }

    private static String formatFailure(Throwable failure) {
        StringBuilder message = new StringBuilder("Java mod callback failed");
        Throwable current = failure;
        int depth = 0;
        while (current != null && depth++ < 8) {
            message.append("\n").append(current.getClass().getName());
            if (current.getMessage() != null) {
                message.append(": ").append(current.getMessage());
            }
            current = current.getCause();
        }
        return message.toString();
    }

    @Override
    public Arguments getArguments() {
        return arguments;
    }

    @Override
    public String[] getLaunchArguments(boolean sanitize) {
        return arguments.toArray();
    }

    @Override
    public boolean canOpenGui() {
        return false;
    }

    @Override
    public boolean hasAwtSupport() {
        return false;
    }
}
