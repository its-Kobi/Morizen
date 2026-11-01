package dev.morizen.compat.provider;

import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Collection;
import java.util.EnumSet;
import java.util.List;
import java.util.Set;
import net.fabricmc.loader.api.metadata.ModMetadata;
import net.fabricmc.loader.impl.game.GameProvider;
import net.fabricmc.loader.impl.game.patch.GameTransformer;
import net.fabricmc.loader.impl.launch.FabricLauncher;
import net.fabricmc.loader.impl.launch.knot.KnotClient;
import net.fabricmc.loader.impl.metadata.BuiltinModMetadata;
import net.fabricmc.loader.impl.util.Arguments;

/** Fabric Loader provider for the native Morizen engine. */
public final class MorizenGameProvider implements GameProvider {
    private static final String BOOTSTRAP_CLASS = "dev.morizen.compat.Bootstrap";
    private static volatile ClassLoader activeTargetLoader;

    private final Arguments arguments = new Arguments();
    private final GameTransformer transformer = new GameTransformer();
    private Path launchDirectory;
    private Path bootstrapJar;

    /** JNI calls this wrapper so loader failures retain their nested cause chain. */
    public static void launchKnot(String[] arguments) {
        try {
            KnotClient.main(arguments);
        } catch (Error fatal) {
            throw fatal;
        } catch (Throwable failure) {
            if (failure instanceof IllegalStateException translated
                    && translated.getMessage() != null
                    && translated.getMessage().startsWith("Java mod callback failed")) {
                throw translated;
            }
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
                new BuiltinMod(paths, metadata("minecraft", "26.3", "Minecraft")),
                new BuiltinMod(paths, metadata("morizen", "0.1.0", "Morizen")));
    }

    private static ModMetadata metadata(String id, String version, String name) {
        return new BuiltinModMetadata.Builder(id, version)
                .setName(name)
                .build();
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
    public boolean requiresUrlClassLoader() {
        return false;
    }

    @Override
    public Set<BuiltinTransform> getBuiltinTransforms(String className) {
        return EnumSet.of(BuiltinTransform.STRIP_ENVIRONMENT, BuiltinTransform.CLASS_TWEAKS);
    }

    @Override
    public boolean isEnabled() {
        return true;
    }

    @Override
    public boolean locateGame(FabricLauncher launcher, String[] launchArguments) {
        arguments.parse(launchArguments);
        launchDirectory = requiredPath("morizen.gameDir");
        bootstrapJar = requiredPath("morizen.bootstrapJar");
        if (!Files.isDirectory(launchDirectory)) {
            throw new IllegalStateException("Morizen game directory does not exist: " + launchDirectory);
        }
        if (!Files.isRegularFile(bootstrapJar)) {
            throw new IllegalStateException("Morizen bootstrap JAR does not exist: " + bootstrapJar);
        }
        return true;
    }

    private static Path requiredPath(String property) {
        String value = System.getProperty(property);
        if (value == null || value.isBlank()) {
            throw new IllegalStateException("Missing required system property " + property);
        }
        return Path.of(value).toAbsolutePath().normalize();
    }

    @Override
    public void initialize(FabricLauncher launcher) {
        transformer.locateEntrypoints(launcher, List.of(bootstrapJar));
    }

    @Override
    public GameTransformer getEntrypointTransformer() {
        return transformer;
    }

    @Override
    public void unlockClassPath(FabricLauncher launcher) {
        launcher.addToClassPath(bootstrapJar);
    }

    @Override
    public void launch(ClassLoader targetLoader) {
        activeTargetLoader = targetLoader;
        System.out.println("morizen-java: provider=minecraft class=" + getClass().getName());
        invokeBootstrap(targetLoader, "start", new Class<?>[] {Path.class}, launchDirectory);
    }

    /** Called by the Rust owner thread before it destroys the VM. */
    public static void stopActive() {
        ClassLoader targetLoader = activeTargetLoader;
        if (targetLoader == null) {
            return;
        }
        try {
            invokeBootstrap(targetLoader, "stop", new Class<?>[0]);
        } finally {
            activeTargetLoader = null;
        }
    }

    public static ClassLoader getActiveTargetLoader() {
        ClassLoader targetLoader = activeTargetLoader;
        if (targetLoader == null) {
            throw new IllegalStateException("Fabric target class loader is not active");
        }
        return targetLoader;
    }

    private static void invokeBootstrap(
            ClassLoader targetLoader, String methodName, Class<?>[] parameterTypes, Object... arguments) {
        try {
            Class<?> bootstrap = Class.forName(BOOTSTRAP_CLASS, true, targetLoader);
            Method method = bootstrap.getMethod(methodName, parameterTypes);
            method.invoke(null, arguments);
        } catch (InvocationTargetException error) {
            Throwable cause = error.getCause();
            if (cause instanceof Error fatal) {
                throw fatal;
            }
            throw new IllegalStateException(formatFailure(cause), cause);
        } catch (ReflectiveOperationException error) {
            throw new IllegalStateException("Could not invoke Morizen bootstrap " + methodName, error);
        }
    }

    private static String formatFailure(Throwable failure) {
        StringBuilder message = new StringBuilder("Java mod callback failed");
        Throwable current = failure;
        int depth = 0;
        int frames = 0;
        while (current != null && depth < 8) {
            message.append("\n").append(depth == 0 ? "thrown: " : "caused by: ")
                    .append(current.getClass().getName());
            if (current.getMessage() != null) {
                message.append(": ").append(current.getMessage());
            }
            for (StackTraceElement element : current.getStackTrace()) {
                if (frames++ == 32) {
                    message.append("\n  ... stack truncated");
                    return message.toString();
                }
                message.append("\n  at ").append(element);
            }
            current = current.getCause();
            depth++;
        }
        if (current != null) {
            message.append("\n... cause chain truncated");
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
    public String getRuntimeNamespace(String gameId) {
        return "official";
    }

    @Override
    public String getDefaultModDistributionNamespace(String gameId) {
        return "official";
    }

    @Override
    public boolean canOpenErrorGui() {
        return false;
    }

    @Override
    public boolean hasAwtSupport() {
        return false;
    }
}
